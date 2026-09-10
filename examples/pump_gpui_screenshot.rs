//! Render Pump through the live native GPUI host and write PNG captures.

#[cfg(target_os = "macos")]
mod macos {

    use cocoa::appkit::{NSApp, NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSAutoreleasePool, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize};
    use image::{imageops, ImageFormat, RgbaImage};
    use objc::runtime::{Object, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use pump::gui_gpui::{new_screenshot_gui_with_params, WINDOW_HEIGHT, WINDOW_WIDTH};
    use raw_window_handle::{AppKitWindowHandle, RawWindowHandle};
    use std::path::{Path, PathBuf};
    use std::ptr::NonNull;
    use std::thread;
    use std::time::{Duration, Instant};

    const CAPTURE_WIDTH: u32 = WINDOW_WIDTH;
    const CAPTURE_HEIGHT: u32 = WINDOW_HEIGHT;
    const COMMAND: u64 = 1_u64 << 20;
    const SHIFT: u64 = 1_u64 << 17;

    struct Fixture {
        window: id,
        view: id,
    }

    impl Fixture {
        unsafe fn new(width: u32, height: u32) -> Self {
            let _ = NSApp();
            let frame = NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(f64::from(width), f64::from(height)),
            );
            let window = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
                frame,
                NSWindowStyleMask::NSBorderlessWindowMask,
                NSBackingStoreType::NSBackingStoreBuffered,
                false,
            );
            let view = NSView::alloc(nil).initWithFrame_(frame);
            window.setContentView_(view);
            window.makeKeyAndOrderFront_(nil);
            Self { window, view }
        }

        fn parent_handle(&self) -> RawWindowHandle {
            let mut handle = AppKitWindowHandle::empty();
            handle.ns_view = NonNull::new(self.view as *mut _)
                .expect("fixture view pointer")
                .as_ptr();
            RawWindowHandle::AppKit(handle)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            unsafe {
                let _: () = msg_send![self.window, setContentView: nil];
                let _: () = msg_send![self.view, release];
                let _: () = msg_send![self.window, release];
            }
        }
    }

    unsafe fn pump_appkit(app: id, gui: &toybox::gpui_gui::GpuiHostedGui, seconds: f64) {
        let deadline = Instant::now() + Duration::from_secs_f64(seconds);
        while Instant::now() < deadline {
            let date: id = msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.004_f64];
            let event: id = msg_send![
                app,
                nextEventMatchingMask: usize::MAX
                untilDate: date
                inMode: NSDefaultRunLoopMode
                dequeue: YES
            ];
            if !event.is_null() {
                let _: () = msg_send![app, sendEvent: event];
            }
            let _: () = msg_send![app, updateWindows];
            gui.pump();
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn output_root() -> PathBuf {
        std::env::var_os("TOYBOX_UI_SCREENSHOT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/ui-screenshots"))
            .join("pump")
    }

    fn write_capture(
        root: &Path,
        name: &str,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        output_width: u32,
        output_height: u32,
    ) {
        let expected = usize::try_from(width).unwrap() * usize::try_from(height).unwrap() * 4;
        assert_eq!(
            pixels.len(),
            expected,
            "GPUI capture has invalid RGBA length"
        );
        let image = RgbaImage::from_raw(width, height, pixels).expect("valid GPUI RGBA capture");
        let image = if width == output_width && height == output_height {
            image
        } else {
            imageops::resize(
                &image,
                output_width,
                output_height,
                imageops::FilterType::Lanczos3,
            )
        };
        image
            .save_with_format(root.join(format!("{name}.png")), ImageFormat::Png)
            .expect("screenshot should be writable");
    }

    fn capture(
        app: id,
        fixture: &Fixture,
        gui: &toybox::gpui_gui::GpuiHostedGui,
        root: &Path,
        name: &str,
        output_width: u32,
        output_height: u32,
    ) -> (u32, u32, Vec<u8>) {
        unsafe { pump_appkit(app, gui, 0.10) };
        let (width, height, pixels) = gui.capture_rgba().expect("live GPUI capture");
        eprintln!("{name}: captured {width}x{height}");
        if name.contains("curve-segment-proximity") || name.contains("curve-segment-command-hover")
        {
            let blue = pixels
                .chunks_exact(4)
                .filter(|rgba| {
                    rgba[2] > 180
                        && rgba[2].saturating_sub(rgba[0]) > 70
                        && rgba[1].saturating_sub(rgba[0]) > 30
                })
                .count();
            assert!(
                blue > 50,
                "{name}: expected blue segment feedback, got {blue} pixels"
            );
        }
        write_capture(
            root,
            name,
            width,
            height,
            pixels.clone(),
            output_width,
            output_height,
        );
        let _ = fixture;
        (width, height, pixels)
    }

    fn filter_handle_position(
        capture: &(u32, u32, Vec<u8>),
        approximate_x: f64,
        approximate_y: f64,
        context: &str,
    ) -> (f64, f64) {
        let (width, height, pixels) = capture;
        let scale_x = f64::from(*width) / f64::from(CAPTURE_WIDTH);
        let scale_y = f64::from(*height) / f64::from(CAPTURE_HEIGHT);
        let expected_x = approximate_x * scale_x;
        let expected_y = approximate_y * scale_y;
        let radius = 7.5 * scale_x.max(scale_y);
        let ring_inner = 3.0 * scale_x.min(scale_y);
        let ring_inner_squared = ring_inner * ring_inner;
        let ring_outer_squared = radius * radius;
        let search_x = (expected_x - 32.0 * scale_x).max(0.0) as u32
            ..=((expected_x + 32.0 * scale_x).min(f64::from(*width - 1))) as u32;
        let search_y = (expected_y - 36.0 * scale_y).max(0.0) as u32
            ..=((expected_y + 36.0 * scale_y).min(f64::from(*height - 1))) as u32;
        let orange_pixels = search_y
            .clone()
            .flat_map(|y| search_x.clone().map(move |x| (x, y)))
            .filter(|(x, y)| {
                let offset = ((usize::try_from(*y).unwrap() * usize::try_from(*width).unwrap())
                    + usize::try_from(*x).unwrap())
                    * 4;
                let red = pixels[offset];
                let green = pixels[offset + 1];
                let blue = pixels[offset + 2];
                red > 100 && green > 70 && green < 210 && blue < 130 && red > green + 25
            })
            .collect::<Vec<_>>();
        let mut best = None;
        for candidate_y in search_y {
            for candidate_x in search_x.clone() {
                let score = orange_pixels
                    .iter()
                    .filter(|&&(x, y)| {
                        let dx = f64::from(x) - f64::from(candidate_x);
                        let dy = f64::from(y) - f64::from(candidate_y);
                        let distance_squared = dx * dx + dy * dy;
                        distance_squared >= ring_inner_squared
                            && distance_squared <= ring_outer_squared
                    })
                    .count();
                let distance_to_expected = (f64::from(candidate_x) - expected_x).powi(2)
                    + (f64::from(candidate_y) - expected_y).powi(2);
                let replace = best.is_none_or(|(best_score, best_distance, _, _)| {
                    score > best_score
                        || (score == best_score && distance_to_expected < best_distance)
                });
                if replace {
                    best = Some((score, distance_to_expected, candidate_x, candidate_y));
                }
            }
        }
        let (score, _, x, y) = best.unwrap_or((0, 0.0, 0, 0));
        assert!(
            score >= 5,
            "{context}: rendered filter marker not found near ({approximate_x:.1}, {approximate_y:.1}) in {width}x{height} capture (best orange ring score {score})"
        );
        let logical = (f64::from(x) / scale_x, f64::from(y) / scale_y);
        eprintln!(
            "{context}: rendered filter marker at ({:.2}, {:.2}) from {width}x{height} capture (orange ring score {score})",
            logical.0, logical.1
        );
        logical
    }

    fn numeric_label_pixels(capture: &(u32, u32, Vec<u8>), center_x: u32) -> Vec<u8> {
        let (width, height, pixels) = capture;
        let x0 = (center_x - 35) * width / 640;
        let x1 = (center_x + 35) * width / 640;
        let y0 = 357 * height / 400;
        let y1 = 373 * height / 400;
        let mut label = Vec::new();
        for y in y0..y1 {
            let start = ((y * width + x0) * 4) as usize;
            let end = ((y * width + x1) * 4) as usize;
            label.extend_from_slice(&pixels[start..end]);
        }
        label
    }

    fn assert_rendered_bounds(name: &str, width: u32, height: u32, pixels: &[u8]) {
        let expected = usize::try_from(width).unwrap() * usize::try_from(height).unwrap() * 4;
        assert_eq!(pixels.len(), expected, "{name}: invalid RGBA length");
        let sample_x = [width * 3 / 4, width * 7 / 8];
        let sample_y = [height * 3 / 5, height * 4 / 5];
        let visible_samples = sample_y
            .into_iter()
            .flat_map(|y| sample_x.into_iter().map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let offset = (usize::try_from(y).unwrap() * usize::try_from(width).unwrap()
                    + usize::try_from(x).unwrap())
                    * 4;
                pixels[offset..offset + 3]
                    .iter()
                    .copied()
                    .map(u32::from)
                    .sum::<u32>()
                    > 24
            })
            .count();
        assert!(
            visible_samples >= 3,
            "{name}: rendered content does not fill the requested viewport"
        );
    }

    unsafe fn send_mouse_event(
        window: id,
        _width: u32,
        height: u32,
        event_type: usize,
        x: f64,
        top_y: f64,
        modifiers: u64,
    ) {
        let window_number: isize = msg_send![window, windowNumber];
        let location = NSPoint::new(x, f64::from(height) - top_y);
        let event: id = msg_send![
            class!(NSEvent),
            mouseEventWithType: event_type
            location: location
            modifierFlags: modifiers
            timestamp: 0.0_f64
            windowNumber: window_number
            context: std::ptr::null_mut::<Object>()
            eventNumber: 1_isize
            clickCount: 1_isize
            pressure: 1.0_f64
        ];
        if event_type == 5 {
            // WindowServer tracking-area events target their registered owner.
            // A raw NSWindow sendEvent(mouseMoved) instead targets the first
            // responder, so explicitly emulate the documented tracking route.
            let content: id = msg_send![window, contentView];
            let children: id = msg_send![content, subviews];
            let child: id = msg_send![children, lastObject];
            let areas: id = msg_send![child, trackingAreas];
            let area: id = msg_send![areas, firstObject];
            assert!(
                !area.is_null(),
                "native GPUI child must install a tracking area"
            );
            let owner: id = msg_send![area, owner];
            assert_eq!(owner, child, "tracking-area owner must be the GPUI child");
            if x < 0.0
                || x >= f64::from(WINDOW_WIDTH)
                || top_y < 0.0
                || top_y >= f64::from(WINDOW_HEIGHT)
            {
                let _: () = msg_send![owner, mouseExited: event];
            } else {
                let _: () = msg_send![owner, mouseMoved: event];
            }
        } else {
            let _: () = msg_send![window, sendEvent: event];
        }
    }

    unsafe fn send_mouse_move(
        window: id,
        width: u32,
        height: u32,
        x: f64,
        top_y: f64,
        modifiers: u64,
    ) {
        send_mouse_event(window, width, height, 5, x, top_y, modifiers);
    }

    unsafe fn send_mouse_down(
        window: id,
        width: u32,
        height: u32,
        x: f64,
        top_y: f64,
        modifiers: u64,
    ) {
        send_mouse_event(window, width, height, 1, x, top_y, modifiers);
    }

    unsafe fn send_mouse_up(
        window: id,
        width: u32,
        height: u32,
        x: f64,
        top_y: f64,
        modifiers: u64,
    ) {
        send_mouse_event(window, width, height, 2, x, top_y, modifiers);
    }

    unsafe fn send_secondary_mouse_down(
        window: id,
        width: u32,
        height: u32,
        x: f64,
        top_y: f64,
        modifiers: u64,
    ) {
        send_mouse_event(window, width, height, 3, x, top_y, modifiers);
    }

    unsafe fn send_secondary_mouse_dragged(
        window: id,
        width: u32,
        height: u32,
        x: f64,
        top_y: f64,
        modifiers: u64,
    ) {
        send_mouse_event(window, width, height, 7, x, top_y, modifiers);
    }

    unsafe fn send_secondary_mouse_up(
        window: id,
        width: u32,
        height: u32,
        x: f64,
        top_y: f64,
        modifiers: u64,
    ) {
        send_mouse_event(window, width, height, 4, x, top_y, modifiers);
    }

    unsafe fn send_click(window: id, width: u32, height: u32, x: f64, top_y: f64) {
        send_mouse_down(window, width, height, x, top_y, 0);
        send_mouse_up(window, width, height, x, top_y, 0);
    }

    pub fn main() {
        unsafe {
            let _pool = NSAutoreleasePool::new(nil);
            let app = NSApp();
            let fixture = Fixture::new(CAPTURE_WIDTH, CAPTURE_HEIGHT);
            let root = output_root();
            std::fs::create_dir_all(&root).expect("screenshot directory");
            let slot_path = std::env::temp_dir().join(format!(
                "pump-gpui-slots-{}-curve-slots.bin",
                std::process::id()
            ));
            std::env::set_var("PUMP_GLOBAL_CURVE_SLOTS_PATH", &slot_path);
            let (mut gui, params, status) = new_screenshot_gui_with_params();
            gui.set_parent_raw(fixture.parent_handle());
            assert!(gui.open(), "Pump GPUI editor should open");
            gui.request_resize(CAPTURE_WIDTH, CAPTURE_HEIGHT);

            let default_capture =
                capture(app, &fixture, &gui, &root, "pump-default-640x400", 640, 400);

            params.set_filter_enabled(1.0);
            params.set_filter_hp_freq_hz(320.0);
            params.set_filter_hp_q(1.25);
            params.set_filter_lp_freq_hz(7_500.0);
            params.set_filter_lp_q(1.75);
            params.set_filter_hp_slope(1.0);
            params.set_filter_lp_slope(2.0);
            let filter_capture = capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-filter-enabled-640x400",
                640,
                400,
            );
            let (filter_hp_x, filter_hp_y) = filter_handle_position(
                &filter_capture,
                265.0,
                135.0,
                "pump-filter-enabled-640x400",
            );
            send_click(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                filter_hp_x,
                filter_hp_y,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-filter-selected-hp-640x400",
                640,
                400,
            );
            params.set_filter_enabled(0.0);

            let curve_before_seam = params.editable_curve_snapshot();
            let phase_before_seam = params.phase_offset();
            params.set_phase_offset(0.25);
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-curve-seam-offset-640x400",
                640,
                400,
            );
            assert_eq!(
                params.editable_curve_snapshot(),
                curve_before_seam,
                "offset must only project seam handles, not change authored nodes"
            );
            params.set_phase_offset(phase_before_seam);

            // Curve feedback captures are driven through native AppKit
            // pointer/modifier events so they exercise the same admission and
            // retained-state paths as a hosted plug-in editor.
            send_mouse_move(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                95.0,
                217.0,
                0,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-curve-node-hover-640x400",
                640,
                400,
            );
            send_mouse_move(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                170.0,
                185.0,
                0,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-curve-segment-proximity-640x400",
                640,
                400,
            );
            send_mouse_move(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                170.0,
                196.0,
                COMMAND,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-curve-segment-command-hover-640x400",
                640,
                400,
            );
            send_mouse_move(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                170.0,
                196.0,
                0,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-curve-insertion-preview-640x400",
                640,
                400,
            );
            send_mouse_down(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                58.0,
                245.0,
                COMMAND | SHIFT,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-curve-offset-active-640x400",
                640,
                400,
            );
            send_mouse_move(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                100.0,
                245.0,
                COMMAND | SHIFT,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-curve-offset-sliding-640x400",
                640,
                400,
            );
            send_mouse_up(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                100.0,
                245.0,
                COMMAND | SHIFT,
            );
            send_mouse_down(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                300.0,
                100.0,
                SHIFT,
            );
            send_mouse_move(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                400.0,
                210.0,
                SHIFT,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-curve-marquee-active-640x400",
                640,
                400,
            );
            send_mouse_up(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                400.0,
                210.0,
                SHIFT,
            );
            send_secondary_mouse_down(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                300.0,
                90.0,
                0,
            );
            send_secondary_mouse_dragged(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                340.0,
                200.0,
                0,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-curve-paint-preview-640x400",
                640,
                400,
            );
            send_secondary_mouse_up(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                340.0,
                200.0,
                0,
            );

            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-header-normal-640x400",
                640,
                400,
            );

            // Header captures are driven by native AppKit mouse events, so the
            // hover and pressed states are produced by the same GPUI hit testing
            // path used by a host window.
            send_mouse_move(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                255.0,
                32.0,
                0,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-header-hovered-640x400",
                640,
                400,
            );
            send_mouse_move(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                351.0,
                32.0,
                0,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-header-copy-hovered-640x400",
                640,
                400,
            );
            send_mouse_move(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                319.0,
                32.0,
                0,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-header-a-hovered-640x400",
                640,
                400,
            );
            send_mouse_move(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                383.0,
                32.0,
                0,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-header-b-hovered-640x400",
                640,
                400,
            );
            send_mouse_down(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                255.0,
                32.0,
                0,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-header-pressed-640x400",
                640,
                400,
            );
            send_mouse_up(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                255.0,
                32.0,
                0,
            );
            send_mouse_move(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 20.0, 20.0, 0);
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-header-disabled-640x400",
                640,
                400,
            );

            let initial_sound = params.active_sound();
            send_click(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 319.0, 32.0);
            assert_eq!(
                params.active_sound(),
                initial_sound,
                "native A click should select A"
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-header-a-active-640x400",
                640,
                400,
            );
            send_click(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 383.0, 32.0);
            assert_ne!(
                params.active_sound(),
                initial_sound,
                "native B click should select B"
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-header-b-active-640x400",
                640,
                400,
            );

            params.set_bypass(1.0);
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-bypass-bypassed-640x400",
                640,
                400,
            );
            params.set_bypass(0.0);
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-bypass-active-640x400",
                640,
                400,
            );

            gui.request_resize(1280, 800);
            let (max_width, max_height, max_pixels) =
                capture(app, &fixture, &gui, &root, "pump-max-1280x800", 1280, 800);
            assert_rendered_bounds("pump-max-1280x800", max_width, max_height, &max_pixels);
            gui.request_resize(640, 400);
            capture(app, &fixture, &gui, &root, "pump-min-640x400", 640, 400);

            // Keep the responsive reference explicit: Toybox owns the native
            // display scale, while this capture is a live 800x500 logical raster.
            gui.request_resize(800, 500);
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-responsive-800x500",
                800,
                500,
            );
            gui.request_resize(640, 400);

            // Keep the extra state captures tied to real Pump interactions. The
            // old generic component gallery was a Radiant-only contract and did
            // not exercise a production Pump view.
            send_click(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 195.0, 39.0);
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-numeric-focused-640x400",
                640,
                400,
            );
            send_mouse_down(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                398.0,
                333.0,
                0,
            );
            send_mouse_move(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                410.0,
                320.0,
                0,
            );
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-knob-drag-640x400",
                640,
                400,
            );
            send_mouse_up(
                fixture.window,
                CAPTURE_WIDTH,
                CAPTURE_HEIGHT,
                410.0,
                320.0,
                0,
            );

            params.set_mix(0.62);
            params.set_smooth(0.35);
            params.set_swing(0.4);
            params.set_output_gain_db(-3.0);
            status.publish_gain_reduction(0.5, true);
            pump::gui_gpui::seed_screenshot_waveform(&status);
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-waveform-layers-640x400",
                640,
                400,
            );
            let host_updated_capture = capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-non-default-active-meter-640x400",
                640,
                400,
            );

            for (name, center_x) in [
                ("Smooth", 87),
                ("Swing", 241),
                ("Mix", 397),
                ("Output", 551),
            ] {
                assert_ne!(
                    numeric_label_pixels(&default_capture, center_x),
                    numeric_label_pixels(&host_updated_capture, center_x),
                    "idle host {name} update must repaint its numeric label without clicking"
                );
            }

            params.set_timing_mode(1.0);
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-free-rate-640x400",
                640,
                400,
            );
            params.set_timing_mode(0.0);

            // Reopen exercises the same native child lifecycle used by plugin
            // hosts and ensures the second frame is still a live GPUI scene.
            gui.close();
            gui.set_parent_raw(fixture.parent_handle());
            assert!(gui.open(), "Pump GPUI editor should reopen");
            gui.request_resize(640, 400);
            capture(
                app,
                &fixture,
                &gui,
                &root,
                "pump-fingerprint-second",
                640,
                400,
            );
            gui.close();
            let _ = std::fs::remove_file(slot_path);
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    macos::main();
}

#[cfg(not(target_os = "macos"))]
fn main() {}
