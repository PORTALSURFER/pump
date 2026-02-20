    use std::sync::Arc;

    use toybox::gui::{screenshot_harness, Size};

    use super::{AutomationQueue, GuiState, GuiStatus, PumpParams, WINDOW_HEIGHT, WINDOW_WIDTH};

    #[test]
    fn screenshot_renders_initial_ui() {
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        let queue = Arc::new(AutomationQueue::default());
        let state = GuiState::new(params, status, queue, None);

        screenshot_harness::capture_initial_ui_screenshots_if_enabled(
            env!("CARGO_PKG_NAME"),
            Size {
                width: WINDOW_WIDTH,
                height: WINDOW_HEIGHT,
            },
            |input| state.build_ui(input),
        )
        .expect("failed to capture pump headless screenshots");
    }
