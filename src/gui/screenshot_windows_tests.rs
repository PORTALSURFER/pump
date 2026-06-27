    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crate::{
        params::{MAX_PRESETS, SYNC_DIVISIONS},
        GuiStatus,
    };
    use toybox::clap::gui::InputState;
    use toybox::gui::declarative::{
        LayoutDiagnosticsMode, LayoutNodeDiagnostic, LayoutNodeKind, UiAction,
    };
    use toybox::gui::{Point, Rect, Size, render_spec_to_frame, screenshot_harness};
    use toybox::raw_window_handle::{RawWindowHandle, Win32WindowHandle};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SendMessageW, ShowWindow, SW_SHOW, WM_LBUTTONDOWN,
        WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WS_OVERLAPPEDWINDOW,
    };
    use windows::core::w;

    use super::{
        resolve_vertical_slot_heights, AutomationQueue, GuiState, PumpGui, PumpParams, CURVE_KEY,
        DIVISION_KEY, PRESET_DROPDOWN_KEY, WINDOW_HEIGHT, WINDOW_WIDTH,
    };

    const MIN_VISUAL_CHANGE_PIXELS: usize = 24;
    const MIN_CURVE_OVERLAY_CHANGE_PIXELS: usize = 24;

    struct CapturedFrame {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    }

    struct DropdownGeometry {
        preset_dropdown: Rect,
        division_dropdown: Rect,
        curve_region: Rect,
    }

    #[test]
    fn screenshot_renders_initial_ui() {
        if !screenshot_harness::screenshots_enabled() {
            return;
        }

        let sizes = screenshot_harness::default_screenshot_sizes(Size {
            width: WINDOW_WIDTH,
            height: WINDOW_HEIGHT,
        });
        for size in sizes {
            render_and_capture_at_size(size.width, size.height);
        }
    }

    #[test]
    fn dropdown_popups_remain_usable_over_curve_content() {
        let width = WINDOW_WIDTH;
        let height = WINDOW_HEIGHT;
        let params = Arc::new(PumpParams::new());
        for _ in 1..MAX_PRESETS {
            params
                .add_preset_from_current_state()
                .expect("preset insertion should succeed");
        }
        params.load_preset(0).expect("preset selection should succeed");
        let status = Arc::new(GuiStatus::default());
        let queue = Arc::new(AutomationQueue::default());
        let mut state = GuiState::new(
            Arc::clone(&params),
            status,
            Arc::clone(&queue),
            None,
        );

        let geometry = dropdown_geometry_for_size(width, height)
            .expect("dropdown geometry should resolve from diagnostics");
        let baseline = render_state_frame(&state, width, height)
            .expect("baseline frame should render");
        let curve_variation = pixel_variation_count_in_rect(
            baseline.width,
            baseline.height,
            &baseline.pixels,
            geometry.curve_region,
        );
        assert!(
            curve_variation > 0,
            "curve region should contain rendered content under dropdown overlap area"
        );

        let preset_menu = dropdown_menu_rect(
            geometry.preset_dropdown,
            params.preset_bank_snapshot().presets.len(),
            false,
            height,
        );
        assert!(
            rects_intersect(preset_menu, geometry.curve_region),
            "preset dropdown popup geometry should overlap curve region"
        );
        assert!(
            rect_contains_point(
                preset_menu,
                dropdown_menu_row_point(geometry.preset_dropdown, 1, false),
            ),
            "preset popup geometry should include at least one selectable row"
        );

        let selected_preset_before = params.preset_bank_snapshot().selected;
        state.reduce_action(UiAction::DropdownSelected {
            key: PRESET_DROPDOWN_KEY.to_string(),
            index: 1,
        });
        let after_preset_select = render_state_frame(&state, width, height)
            .expect("preset selection frame should render");
        assert!(
            changed_pixel_count(&baseline.pixels, &after_preset_select.pixels)
                >= MIN_VISUAL_CHANGE_PIXELS,
            "preset dropdown selection should update frame"
        );
        let selected_preset_after = params.preset_bank_snapshot().selected;
        assert_ne!(
            selected_preset_after, selected_preset_before,
            "preset dropdown selection must remain interactive"
        );

        let division_before = params.sync_division();
        let division_menu = dropdown_menu_rect(
            geometry.division_dropdown,
            SYNC_DIVISIONS.len(),
            true,
            height,
        );
        assert!(
            rects_intersect(division_menu, geometry.curve_region),
            "division dropdown popup should overlap visible curve content"
        );
        assert!(
            rect_contains_point(
                division_menu,
                dropdown_menu_row_point(geometry.division_dropdown, 1, true),
            ),
            "division popup geometry should include at least one selectable row over the curve"
        );
        state.reduce_action(UiAction::DropdownSelected {
            key: DIVISION_KEY.to_string(),
            index: 1,
        });
        let after_division_select = render_state_frame(&state, width, height)
            .expect("division selection frame should render");
        let curve_overlay_pixels = changed_pixel_count_in_rect(
            &after_preset_select.pixels,
            &after_division_select.pixels,
            width,
            height,
            geometry.curve_region,
        );
        assert!(
            curve_overlay_pixels >= MIN_CURVE_OVERLAY_CHANGE_PIXELS,
            "division dropdown selection should update visible curve-region frame content"
        );
        let division_after = params.sync_division();
        assert_ne!(
            division_after, division_before,
            "division dropdown selection must remain interactive"
        );
    }

    fn render_and_capture_at_size(width: u32, height: u32) {
        let mut gui = PumpGui::default();
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        let host = parent_window(width.max(WINDOW_WIDTH), height.max(WINDOW_HEIGHT));
        let queue = Arc::new(AutomationQueue::default());

        gui.set_parent_raw(host.raw_handle());
        gui.open(&params, &status, queue, None)
            .expect("open should succeed");
        gui.window.show();
        let _ = wait_for_window_handle(&gui, host.hwnd);
        let _ = wait_for_any_logical_size(&gui);
        gui.request_resize(width, height);
        wait_for_requested_logical_size(&gui, width, height)
            .expect("plugin GUI did not reach requested logical size");

        let frame =
            wait_for_rendered_frame(&gui, width, height).expect("failed to capture rendered frame");
        let path = screenshot_path(env!("CARGO_PKG_NAME"), width, height);
        screenshot_harness::write_png_rgba8(&path, frame.width, frame.height, frame.pixels)
            .expect("failed to write screenshot PNG");

        gui.close();
        assert!(path.exists());
    }

    fn dropdown_geometry_for_size(width: u32, height: u32) -> Result<DropdownGeometry, String> {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        let frame = render_spec_to_frame(Size { width, height }, |input: &InputState| {
            let mut spec = state.build_ui(input);
            spec.root.layout_diagnostics_mode = LayoutDiagnosticsMode::PerNode;
            spec
        })?;
        let diagnostics = frame.render_result.node_layout_diagnostics;
        let preset_dropdown =
            find_dropdown_rect(&diagnostics, PRESET_DROPDOWN_KEY).ok_or_else(|| {
                "missing preset-dropdown diagnostics rectangle".to_string()
            })?;
        let division_dropdown = find_dropdown_rect(&diagnostics, DIVISION_KEY)
            .ok_or_else(|| "missing division diagnostics rectangle".to_string())?;
        let curve_region = find_curve_rect(&diagnostics).unwrap_or_else(|| {
            let (header_h, curve_h, _, _) = resolve_vertical_slot_heights(height);
            Rect {
                origin: Point {
                    x: 0,
                    y: header_h as i32,
                },
                size: Size {
                    width,
                    height: curve_h,
                },
            }
        });
        Ok(DropdownGeometry {
            preset_dropdown,
            division_dropdown,
            curve_region,
        })
    }

    fn find_dropdown_rect(entries: &[LayoutNodeDiagnostic], key: &str) -> Option<Rect> {
        let needle = format!("dropdown:{key}[");
        entries
            .iter()
            .find(|entry| {
                entry.node_kind == LayoutNodeKind::Dropdown && entry.node_path.contains(&needle)
            })
            .map(|entry| entry.resolved_rect)
    }

    fn find_curve_rect(entries: &[LayoutNodeDiagnostic]) -> Option<Rect> {
        let needle = format!("curve-editor:{CURVE_KEY}[");
        entries
            .iter()
            .find(|entry| {
                entry.node_kind == LayoutNodeKind::CurveEditor && entry.node_path.contains(&needle)
            })
            .map(|entry| entry.resolved_rect)
    }

    fn rect_center(rect: Rect) -> Point {
        Point {
            x: rect.origin.x + (rect.size.width as i32 / 2),
            y: rect.origin.y + (rect.size.height as i32 / 2),
        }
    }

    fn dropdown_menu_row_point(control_rect: Rect, row_index: usize, open_up: bool) -> Point {
        let row_height = control_rect.size.height as i32;
        let menu_row = row_index as i32 + 1;
        let y = if open_up {
            control_rect.origin.y - row_height * menu_row + (row_height / 2)
        } else {
            control_rect.origin.y + row_height * menu_row + (row_height / 2)
        };
        Point {
            x: control_rect.origin.x + (control_rect.size.width as i32 / 2),
            y,
        }
    }

    fn render_state_frame(
        state: &GuiState,
        width: u32,
        height: u32,
    ) -> Result<CapturedFrame, String> {
        let frame = render_spec_to_frame(Size { width, height }, |input: &InputState| {
            state.build_ui(input)
        })?;
        Ok(CapturedFrame {
            width: frame.width,
            height: frame.height,
            pixels: frame.pixels,
        })
    }

    fn dropdown_menu_rect(
        control_rect: Rect,
        option_count: usize,
        open_up: bool,
        window_height: u32,
    ) -> Rect {
        let row_height = control_rect.size.height.max(1) as i32;
        let unclamped_height = row_height.saturating_mul(option_count.max(1) as i32);
        let menu_height = unclamped_height.min(window_height as i32);
        let origin_y = if open_up {
            (control_rect.origin.y - menu_height).max(0)
        } else {
            let max_origin = window_height as i32 - menu_height;
            (control_rect.origin.y + row_height).min(max_origin.max(0))
        };
        Rect {
            origin: Point {
                x: control_rect.origin.x,
                y: origin_y,
            },
            size: Size {
                width: control_rect.size.width,
                height: menu_height.max(0) as u32,
            },
        }
    }

    fn rects_intersect(left: Rect, right: Rect) -> bool {
        let left_x1 = left.origin.x + left.size.width as i32;
        let left_y1 = left.origin.y + left.size.height as i32;
        let right_x1 = right.origin.x + right.size.width as i32;
        let right_y1 = right.origin.y + right.size.height as i32;
        left.origin.x < right_x1
            && left_x1 > right.origin.x
            && left.origin.y < right_y1
            && left_y1 > right.origin.y
    }

    fn rect_contains_point(rect: Rect, point: Point) -> bool {
        point.x >= rect.origin.x
            && point.y >= rect.origin.y
            && point.x < rect.origin.x + rect.size.width as i32
            && point.y < rect.origin.y + rect.size.height as i32
    }

    fn pixel_variation_count_in_rect(
        width: u32,
        height: u32,
        pixels: &[u8],
        rect: Rect,
    ) -> usize {
        let Some((x0, y0, x1, y1)) = clamp_rect(rect, width, height) else {
            return 0;
        };
        let stride = width as usize * 4;
        let mut unique = Vec::<[u8; 4]>::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let offset = y as usize * stride + x as usize * 4;
                let pixel = [
                    pixels[offset],
                    pixels[offset + 1],
                    pixels[offset + 2],
                    pixels[offset + 3],
                ];
                if !unique.contains(&pixel) {
                    unique.push(pixel);
                    if unique.len() > 1 {
                        return unique.len();
                    }
                }
            }
        }
        unique.len()
    }

    fn parent_window(width: u32, height: u32) -> ScreenshotParentWindow {
        ScreenshotParentWindow::new(width, height)
    }

    fn wait_for_window_handle(gui: &PumpGui, expected_parent: HWND) -> HWND {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Some(handle) = gui.window.handle() {
                if handle.is_valid() && handle.parent_matches(expected_parent.0 as isize) {
                    return handle.hwnd();
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("plugin GUI handle did not become available under expected parent");
    }

    fn wait_for_any_logical_size(gui: &PumpGui) -> (u32, u32) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Some(size) = gui.last_size() {
                if size.0 > 0 && size.1 > 0 {
                    return size;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("plugin GUI never reported any logical size");
    }

    fn wait_for_requested_logical_size(gui: &PumpGui, width: u32, height: u32) -> Result<(), String> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Some((current_w, current_h)) = gui.last_size() {
                if current_w == width && current_h == height {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(format!(
            "plugin GUI never reached requested logical size ({width}, {height})"
        ))
    }

    fn screenshot_path(plugin: &str, width: u32, height: u32) -> PathBuf {
        screenshot_harness::screenshot_output_path(plugin, width, height)
            .expect("resolve screenshot path")
    }

    fn wait_for_rendered_frame(
        gui: &PumpGui,
        expected_width: u32,
        expected_height: u32,
    ) -> Result<CapturedFrame, String> {
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut last_uniform: Option<CapturedFrame> = None;
        let mut last_error: Option<String> = None;

        while Instant::now() < deadline {
            match gui.window.capture_next_frame(Duration::from_millis(500)) {
                Ok(frame) => {
                    if frame.width != expected_width || frame.height != expected_height {
                        last_error = Some(format!(
                            "captured unexpected frame size {}x{}, expected {}x{}",
                            frame.width, frame.height, expected_width, expected_height
                        ));
                    } else if frame_has_non_uniform_content(&frame.pixels) {
                        return Ok(CapturedFrame {
                            width: frame.width,
                            height: frame.height,
                            pixels: frame.pixels,
                        });
                    } else {
                        last_uniform = Some(CapturedFrame {
                            width: frame.width,
                            height: frame.height,
                            pixels: frame.pixels,
                        });
                    }
                }
                Err(err) => last_error = Some(err),
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        if let Some(frame) = last_uniform {
            return Err(format!(
                "captured only uniform/blank frames at {}x{} before timeout",
                frame.width, frame.height
            ));
        }
        if let Some(err) = last_error {
            return Err(format!("failed to capture non-uniform frame before timeout: {err}"));
        }
        Err("failed to capture any frame before timeout".to_string())
    }

    include!("screenshot_windows_interaction_helpers.rs");
