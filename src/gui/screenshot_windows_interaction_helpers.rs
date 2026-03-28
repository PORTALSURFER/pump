    fn wait_for_changed_frame(
        gui: &PumpGui,
        expected_width: u32,
        expected_height: u32,
        reference: &CapturedFrame,
        min_changed_pixels: usize,
        label: &str,
    ) -> Result<CapturedFrame, String> {
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut last_error: Option<String> = None;

        while Instant::now() < deadline {
            match gui.window.capture_next_frame(Duration::from_millis(500)) {
                Ok(frame) => {
                    if frame.width != expected_width || frame.height != expected_height {
                        last_error = Some(format!(
                            "{label}: captured unexpected frame size {}x{}, expected {}x{}",
                            frame.width, frame.height, expected_width, expected_height
                        ));
                    } else {
                        let changed = changed_pixel_count(&reference.pixels, &frame.pixels);
                        if changed >= min_changed_pixels {
                            return Ok(CapturedFrame {
                                width: frame.width,
                                height: frame.height,
                                pixels: frame.pixels,
                            });
                        }
                    }
                }
                Err(err) => last_error = Some(format!("{label}: {err}")),
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        if let Some(err) = last_error {
            return Err(err);
        }
        Err(format!(
            "{label}: no frame reached minimum visual delta ({min_changed_pixels} pixels)"
        ))
    }

    fn changed_pixel_count(left: &[u8], right: &[u8]) -> usize {
        if left.len() != right.len() {
            return usize::MAX;
        }
        left.chunks_exact(4)
            .zip(right.chunks_exact(4))
            .filter(|(l, r)| l != r)
            .count()
    }

    fn changed_pixel_count_in_rect(
        left: &[u8],
        right: &[u8],
        width: u32,
        height: u32,
        rect: Rect,
    ) -> usize {
        if left.len() != right.len() {
            return usize::MAX;
        }
        let clamped = clamp_rect(rect, width, height);
        let Some((x0, y0, x1, y1)) = clamped else {
            return 0;
        };
        let stride = width as usize * 4;
        let mut changed = 0usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let offset = y as usize * stride + x as usize * 4;
                if left[offset..offset + 4] != right[offset..offset + 4] {
                    changed += 1;
                }
            }
        }
        changed
    }

    fn clamp_rect(rect: Rect, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
        let x0 = rect.origin.x.max(0) as u32;
        let y0 = rect.origin.y.max(0) as u32;
        let x1 = (rect.origin.x + rect.size.width as i32).clamp(0, width as i32) as u32;
        let y1 = (rect.origin.y + rect.size.height as i32).clamp(0, height as i32) as u32;
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        Some((x0, y0, x1, y1))
    }

    fn send_mouse_move(hwnd: HWND, point: Point) {
        let lparam = LPARAM(pack_mouse_lparam(point));
        unsafe {
            let _ = SendMessageW(hwnd, WM_MOUSEMOVE, Some(WPARAM(0)), Some(lparam));
        }
    }

    fn send_left_click(hwnd: HWND, point: Point) {
        let lparam = LPARAM(pack_mouse_lparam(point));
        send_mouse_move(hwnd, point);
        unsafe {
            let _ = SendMessageW(hwnd, WM_LBUTTONDOWN, Some(WPARAM(1)), Some(lparam));
            let _ = SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0)), Some(lparam));
        }
    }

    fn send_mouse_wheel(hwnd: HWND, point: Point, delta: i16) {
        let lparam = LPARAM(pack_mouse_lparam(point));
        let wparam = WPARAM(((delta as u16 as usize) << 16) & 0xFFFF_0000usize);
        unsafe {
            let _ = SendMessageW(hwnd, WM_MOUSEWHEEL, Some(wparam), Some(lparam));
        }
    }

    fn pack_mouse_lparam(point: Point) -> isize {
        let x = point.x as i16 as u16 as u32;
        let y = point.y as i16 as u16 as u32;
        ((y << 16) | x) as isize
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
