    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crate::GuiStatus;
    use toybox::gui::screenshot_harness;
    use toybox::raw_window_handle::{RawWindowHandle, Win32WindowHandle};
    use windows::Win32::Foundation::{HINSTANCE, HWND};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, SW_SHOW, ShowWindow, WS_OVERLAPPEDWINDOW,
    };
    use windows::core::w;

    use super::{AutomationQueue, PumpGui, PumpParams, WINDOW_HEIGHT, WINDOW_WIDTH};

    struct CapturedFrame {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    }

    #[test]
    fn screenshot_renders_initial_ui() {
        if !screenshot_harness::screenshots_enabled() {
            return;
        }

        let sizes = screenshot_harness::default_screenshot_sizes(toybox::gui::Size {
            width: WINDOW_WIDTH,
            height: WINDOW_HEIGHT,
        });
        for size in sizes {
            render_and_capture_at_size(size.width, size.height);
        }
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

    fn frame_has_non_uniform_content(pixels: &[u8]) -> bool {
        if pixels.len() < 8 {
            return false;
        }
        let first = &pixels[0..4];
        for pixel in pixels.chunks_exact(4).skip(1) {
            if pixel != first {
                return true;
            }
        }
        false
    }

    struct ScreenshotParentWindow {
        hwnd: HWND,
    }

    impl ScreenshotParentWindow {
        fn new(width: u32, height: u32) -> Self {
            let hwnd = unsafe {
                CreateWindowExW(
                    Default::default(),
                    w!("STATIC"),
                    w!("toybox-screenshot-host"),
                    WS_OVERLAPPEDWINDOW,
                    0,
                    0,
                    width as i32,
                    height as i32,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .expect("CreateWindowExW failed");

            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }

            Self { hwnd }
        }

        fn raw_handle(&self) -> RawWindowHandle {
            let mut handle = Win32WindowHandle::empty();
            handle.hwnd = self.hwnd.0 as *mut c_void;
            handle.hinstance = HINSTANCE::default().0 as *mut c_void;
            RawWindowHandle::Win32(handle)
        }
    }

    impl Drop for ScreenshotParentWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
