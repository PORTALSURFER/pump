    use std::ffi::c_void;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crate::GuiStatus;
    use toybox::raw_window_handle::{RawWindowHandle, Win32WindowHandle};
    use toybox::gui::screenshot_harness;
    use windows::core::w;
    use windows::Win32::Foundation::{HINSTANCE, HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
        GetWindowDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetClientRect, ShowWindow, SW_SHOW, WS_OVERLAPPEDWINDOW,
    };

    use super::{AutomationQueue, PumpGui, PumpParams, WINDOW_HEIGHT, WINDOW_WIDTH};

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
        let host = parent_window(width, height);
        let queue = Arc::new(AutomationQueue::default());

        gui.set_parent_raw(host.raw_handle());
        gui.open(&params, &status, queue, None)
            .expect("open should succeed");
        let hwnd = wait_for_window_handle(&gui);
        wait_for_any_logical_size(&gui);
        gui.request_resize(width, height);
        wait_for_logical_size(&gui, (width, height));

        std::thread::sleep(Duration::from_millis(75));

        let path = screenshot_path(env!("CARGO_PKG_NAME"), width, height);
        let (captured_width, captured_height) =
            capture_hwnd(hwnd, &path).expect("failed to capture screenshot");
        assert!(
            captured_width == width && captured_height == height,
            "captured image should be exactly {width}x{height}, got {captured_width}x{captured_height}"
        );

        gui.close();
        assert!(path.exists());
    }

    fn parent_window(width: u32, height: u32) -> ScreenshotParentWindow {
        ScreenshotParentWindow::new(width, height)
    }

    fn wait_for_any_logical_size(gui: &PumpGui) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Some(size) = gui.last_size() {
                if size.0 > 0 && size.1 > 0 {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("plugin GUI never reported any logical size");
    }

    fn wait_for_logical_size(gui: &PumpGui, min_size: (u32, u32)) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Some((width, height)) = gui.last_size() {
                if width >= min_size.0 && height >= min_size.1 {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "plugin GUI never reached logical size >= {min_size:?}, last size={:?}",
            gui.last_size()
        );
    }

    fn wait_for_window_handle(gui: &PumpGui) -> HWND {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Some(handle) = gui.window.handle() {
                if handle.is_valid() {
                    return handle.hwnd();
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("plugin GUI handle did not become available");
    }

    fn screenshot_path(plugin: &str, width: u32, height: u32) -> PathBuf {
        screenshot_harness::screenshot_output_path(plugin, width, height)
            .expect("resolve screenshot path")
    }

    fn capture_hwnd(hwnd: HWND, out: &PathBuf) -> Result<(u32, u32), String> {
        let mut client = RECT::default();
        unsafe {
            GetClientRect(hwnd, &mut client as *mut RECT)
                .map_err(|err| format!("GetClientRect failed: {err}"))?;
        };

        let width = u32::try_from(client.right.saturating_sub(client.left))
            .map_err(|_| "invalid client width".to_string())?;
        let height = u32::try_from(client.bottom.saturating_sub(client.top))
            .map_err(|_| "invalid client height".to_string())?;

        if width == 0 || height == 0 {
            return Err("screenshot target has empty geometry".into());
        }

        let source_dc = unsafe { GetWindowDC(Some(hwnd)) };
        if source_dc.is_invalid() {
            return Err("GetWindowDC returned invalid DC".into());
        }

        let memory_dc = unsafe { CreateCompatibleDC(Some(source_dc)) };
        if memory_dc.is_invalid() {
            unsafe {
                let _ = ReleaseDC(Some(hwnd), source_dc);
            }
            return Err("CreateCompatibleDC returned invalid DC".into());
        }

        let bitmap = unsafe { CreateCompatibleBitmap(source_dc, width as i32, height as i32) };
        if bitmap.is_invalid() {
            unsafe {
                let _ = DeleteDC(memory_dc);
                let _ = ReleaseDC(Some(hwnd), source_dc);
            }
            return Err("CreateCompatibleBitmap returned invalid bitmap".into());
        }

        let old_object = unsafe { SelectObject(memory_dc, HGDIOBJ::from(bitmap)) };
        let bitblt_ok = unsafe {
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
        if !bitblt_ok {
            unsafe {
                let _ = SelectObject(memory_dc, old_object);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(memory_dc);
                let _ = ReleaseDC(Some(hwnd), source_dc);
            }
            return Err("BitBlt failed".into());
        }

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

        unsafe {
            let _ = SelectObject(memory_dc, old_object);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory_dc);
            let _ = ReleaseDC(Some(hwnd), source_dc);
        }

        if got == 0 {
            return Err("GetDIBits returned no rows".into());
        }

        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }

        screenshot_harness::write_png_rgba8(out, width, height, pixels)?;
        Ok((width, height))
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
