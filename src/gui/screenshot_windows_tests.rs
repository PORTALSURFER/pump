    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crate::GuiStatus;
    use toybox::gui::screenshot_harness;
    use toybox::raw_window_handle::{RawWindowHandle, Win32WindowHandle};
    use windows::Win32::Foundation::{HINSTANCE, HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetWindowDC, HGDIOBJ,
        ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetClientRect, IsWindowVisible, PW_RENDERFULLCONTENT,
        SW_SHOW, ShowWindow, WS_OVERLAPPEDWINDOW,
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
        let hwnd = wait_for_window_handle(&gui, host.hwnd);
        let _ = wait_for_any_logical_size(&gui);
        gui.request_resize(width, height);
        wait_for_requested_logical_size(&gui, width, height)
            .expect("plugin GUI did not reach requested logical size");
        gui.window.show();
        wait_for_window_visible(hwnd).expect("plugin window never became visible");

        let frame = wait_for_rendered_frame(hwnd).expect("failed to capture rendered screenshot");
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

    fn wait_for_rendered_frame(hwnd: HWND) -> Result<CapturedFrame, String> {
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut last: Option<CapturedFrame> = None;
        let mut last_error: Option<String> = None;

        while Instant::now() < deadline {
            match capture_hwnd_exact(hwnd) {
                Ok(frame) => {
                    if frame_has_non_uniform_content(&frame.pixels) {
                        return Ok(frame);
                    }
                    last = Some(frame);
                }
                Err(err) => {
                    last_error = Some(err);
                }
            }
            std::thread::sleep(Duration::from_millis(40));
        }

        if let Some(frame) = last {
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

    fn capture_hwnd_exact(hwnd: HWND) -> Result<CapturedFrame, String> {
        if let Ok(frame) = capture_with_print_window(hwnd) {
            if frame_has_non_uniform_content(&frame.pixels) {
                return Ok(frame);
            }
        }
        let fallback = capture_with_window_dc(hwnd)?;
        if frame_has_non_uniform_content(&fallback.pixels) {
            return Ok(fallback);
        }
        Err("PrintWindow and GetWindowDC captures were uniform".to_string())
    }

    fn client_rect_size(hwnd: HWND) -> Result<(u32, u32), String> {
        let mut client = RECT::default();
        unsafe {
            GetClientRect(hwnd, &mut client as *mut RECT)
                .map_err(|err| format!("GetClientRect failed: {err}"))?;
        }
        let width = u32::try_from(client.right.saturating_sub(client.left))
            .map_err(|_| "invalid client width".to_string())?;
        let height = u32::try_from(client.bottom.saturating_sub(client.top))
            .map_err(|_| "invalid client height".to_string())?;
        if width == 0 || height == 0 {
            return Err("screenshot target has empty geometry".to_string());
        }
        Ok((width, height))
    }

    fn capture_with_print_window(hwnd: HWND) -> Result<CapturedFrame, String> {
        let (width, height) = client_rect_size(hwnd)?;
        let screen_dc = unsafe { GetDC(None) };
        if screen_dc.is_invalid() {
            return Err("GetDC(None) returned invalid DC".to_string());
        }
        let memory_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
        if memory_dc.is_invalid() {
            unsafe {
                let _ = ReleaseDC(None, screen_dc);
            }
            return Err("CreateCompatibleDC returned invalid DC".to_string());
        }
        let bitmap = unsafe { CreateCompatibleBitmap(screen_dc, width as i32, height as i32) };
        if bitmap.is_invalid() {
            unsafe {
                let _ = DeleteDC(memory_dc);
                let _ = ReleaseDC(None, screen_dc);
            }
            return Err("CreateCompatibleBitmap returned invalid bitmap".to_string());
        }

        let old_object = unsafe { SelectObject(memory_dc, HGDIOBJ::from(bitmap)) };
        let printed = unsafe {
            PrintWindow(
                hwnd,
                memory_dc,
                PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT),
            )
            .as_bool()
        };
        if !printed {
            unsafe {
                let _ = SelectObject(memory_dc, old_object);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(memory_dc);
                let _ = ReleaseDC(None, screen_dc);
            }
            return Err("PrintWindow returned false".to_string());
        }

        let frame = extract_bitmap_rgba(memory_dc, bitmap, width, height)?;
        unsafe {
            let _ = SelectObject(memory_dc, old_object);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory_dc);
            let _ = ReleaseDC(None, screen_dc);
        }
        Ok(frame)
    }

    fn capture_with_window_dc(hwnd: HWND) -> Result<CapturedFrame, String> {
        let (width, height) = client_rect_size(hwnd)?;
        let source_dc = unsafe { GetWindowDC(Some(hwnd)) };
        if source_dc.is_invalid() {
            return Err("GetWindowDC returned invalid DC".to_string());
        }
        let memory_dc = unsafe { CreateCompatibleDC(Some(source_dc)) };
        if memory_dc.is_invalid() {
            unsafe {
                let _ = ReleaseDC(Some(hwnd), source_dc);
            }
            return Err("CreateCompatibleDC returned invalid DC".to_string());
        }
        let bitmap = unsafe { CreateCompatibleBitmap(source_dc, width as i32, height as i32) };
        if bitmap.is_invalid() {
            unsafe {
                let _ = DeleteDC(memory_dc);
                let _ = ReleaseDC(Some(hwnd), source_dc);
            }
            return Err("CreateCompatibleBitmap returned invalid bitmap".to_string());
        }

        let old_object = unsafe { SelectObject(memory_dc, HGDIOBJ::from(bitmap)) };
        let blt_ok = unsafe {
            BitBlt(
                memory_dc,
                0,
                0,
                width as i32,
                height as i32,
                Some(source_dc),
                0,
                0,
                SRCCOPY,
            )
            .is_ok()
        };
        if !blt_ok {
            unsafe {
                let _ = SelectObject(memory_dc, old_object);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(memory_dc);
                let _ = ReleaseDC(Some(hwnd), source_dc);
            }
            return Err("BitBlt(window) failed".to_string());
        }

        let frame = extract_bitmap_rgba(memory_dc, bitmap, width, height)?;
        unsafe {
            let _ = SelectObject(memory_dc, old_object);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory_dc);
            let _ = ReleaseDC(Some(hwnd), source_dc);
        }
        Ok(frame)
    }

    fn wait_for_window_visible(hwnd: HWND) -> Result<(), String> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if unsafe { IsWindowVisible(hwnd).as_bool() } {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err("window did not become visible before timeout".to_string())
    }

    fn extract_bitmap_rgba(
        memory_dc: windows::Win32::Graphics::Gdi::HDC,
        bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
        width: u32,
        height: u32,
    ) -> Result<CapturedFrame, String> {
        let mut bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [Default::default(); 1],
        };

        let width_usize = usize::try_from(width).map_err(|_| "invalid width".to_string())?;
        let height_usize = usize::try_from(height).map_err(|_| "invalid height".to_string())?;
        let pixel_len = width_usize
            .saturating_mul(height_usize)
            .saturating_mul(4);
        let mut pixels = vec![0_u8; pixel_len];

        let got = unsafe {
            GetDIBits(
                memory_dc,
                bitmap,
                0,
                height,
                Some(pixels.as_mut_ptr().cast()),
                &mut bitmap_info,
                DIB_RGB_COLORS,
            )
        };
        if got == 0 {
            return Err("GetDIBits returned no rows".to_string());
        }

        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }

        Ok(CapturedFrame {
            width,
            height,
            pixels,
        })
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
