//! Exercise Pump's native macOS GPUI input path in an isolated AppKit host.

#[cfg(target_os = "macos")]
mod macos {
    use cocoa::appkit::{NSApp, NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSAutoreleasePool, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize};
    use objc::runtime::{Object, BOOL, NO, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use pump::gui_gpui::{new_screenshot_gui_with_params, WINDOW_HEIGHT, WINDOW_WIDTH};
    use raw_window_handle::{AppKitWindowHandle, RawWindowHandle};
    use std::ffi::{c_void, CStr, CString, OsString};
    use std::path::PathBuf;
    use std::ptr::NonNull;
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const OUTPUT_WIDTH: u32 = WINDOW_WIDTH;
    const OUTPUT_HEIGHT: u32 = WINDOW_HEIGHT;
    const COMMAND: u64 = 1_u64 << 20;
    const OPTION: u64 = 1_u64 << 19;
    const SHIFT: u64 = 1_u64 << 17;

    const DELAY_X: f64 = 193.0;
    const DELAY_Y: f64 = 39.0;
    const TIMING_MODE_X: f64 = 37.0;
    const TIMING_VALUE_X: f64 = 115.0;
    const TIMING_VALUE_Y: f64 = 27.0;
    const TIMING_OPTION_FIRST_CENTER_Y: f64 = 66.0;
    const TIMING_OPTION_STEP_Y: f64 = 25.0;
    const SMOOTH_X: f64 = 87.0;
    const SMOOTH_KNOB_Y: f64 = 332.0;
    const SMOOTH_VALUE_Y: f64 = 365.0;
    const BYPASS_X: f64 = 570.0;
    const BYPASS_Y: f64 = 383.0;
    const CURVE_ENDPOINT_X: f64 = 52.0;
    const CURVE_ENDPOINT_Y: f64 = 63.0;
    const CURVE_NODE_X: f64 = 95.0;
    const CURVE_NODE_Y: f64 = 217.0;
    const CURVE_SEGMENT_X: f64 = 170.0;
    const CURVE_SEGMENT_Y: f64 = 185.0;
    const CURVE_PLOT_X: f64 = 300.0;
    const CURVE_PLOT_Y: f64 = 90.0;
    // At the fixed 640x400 contract, the filter plot maps 320 Hz to this
    // rendered handle position. Q=.25 places the handle on the lower guide
    // edge, which gives the native fixture a stable hit target.
    const FILTER_HP_HANDLE_X: f64 = 265.0;
    const FILTER_HANDLE_MIN_Q_Y: f64 = 232.0;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreateScrollWheelEvent2(
            source: *mut c_void,
            units: u32,
            wheel_count: u32,
            wheel1: i32,
            wheel2: i32,
            wheel3: i32,
        ) -> *mut c_void;
        fn CGEventSetLocation(event: *mut c_void, location: NSPoint);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: *const c_void);
    }

    struct NativeFixture {
        window: id,
        view: id,
    }

    impl NativeFixture {
        unsafe fn new() -> Self {
            let _ = NSApp();
            let frame = NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(f64::from(OUTPUT_WIDTH), f64::from(OUTPUT_HEIGHT)),
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

        unsafe fn hosted_view(&self) -> id {
            let subviews: id = msg_send![self.view, subviews];
            let count: usize = msg_send![subviews, count];
            assert!(count > 0, "Pump GPUI should attach a native child view");
            msg_send![subviews, objectAtIndex: count - 1]
        }
    }

    impl Drop for NativeFixture {
        fn drop(&mut self) {
            unsafe {
                let _: () = msg_send![self.window, setContentView: nil];
                let _: () = msg_send![self.view, release];
                let _: () = msg_send![self.window, release];
            }
        }
    }

    struct CurveSlotSandbox {
        directory: PathBuf,
        previous: Option<OsString>,
    }

    impl CurveSlotSandbox {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos();
            let directory = std::env::temp_dir()
                .join(format!("pump-gpui-input-{}-{stamp}", std::process::id()));
            std::fs::create_dir_all(&directory).expect("curve slot sandbox directory");
            let previous = std::env::var_os("PUMP_GLOBAL_CURVE_SLOTS_PATH");
            std::env::set_var(
                "PUMP_GLOBAL_CURVE_SLOTS_PATH",
                directory.join("curve-slots.bin"),
            );
            Self {
                directory,
                previous,
            }
        }
    }

    impl Drop for CurveSlotSandbox {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var("PUMP_GLOBAL_CURVE_SLOTS_PATH", value),
                None => std::env::remove_var("PUMP_GLOBAL_CURVE_SLOTS_PATH"),
            }
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    struct PasteboardSnapshot {
        items: Vec<PasteboardItemSnapshot>,
    }

    struct PasteboardItemSnapshot {
        types: Vec<(String, Vec<u8>)>,
    }

    struct PasteboardRestore(PasteboardSnapshot);

    impl PasteboardRestore {
        unsafe fn capture() -> Self {
            Self(PasteboardSnapshot::capture())
        }
    }

    impl Drop for PasteboardRestore {
        fn drop(&mut self) {
            unsafe {
                self.0.restore();
            }
        }
    }

    impl PasteboardSnapshot {
        unsafe fn capture() -> Self {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            if pasteboard.is_null() {
                return Self { items: Vec::new() };
            }
            let pasteboard_items: id = msg_send![pasteboard, pasteboardItems];
            if pasteboard_items.is_null() {
                return Self { items: Vec::new() };
            }
            let item_count: usize = msg_send![pasteboard_items, count];
            let mut items = Vec::with_capacity(item_count);
            for item_index in 0..item_count {
                let item: id = msg_send![pasteboard_items, objectAtIndex: item_index];
                if item.is_null() {
                    continue;
                }
                let item_types: id = msg_send![item, types];
                if item_types.is_null() {
                    items.push(PasteboardItemSnapshot { types: Vec::new() });
                    continue;
                }
                let type_count: usize = msg_send![item_types, count];
                let mut types = Vec::with_capacity(type_count);
                for type_index in 0..type_count {
                    let type_object: id = msg_send![item_types, objectAtIndex: type_index];
                    let type_name =
                        ns_string(type_object).expect("pasteboard type should expose UTF-8 text");
                    let data: id = msg_send![item, dataForType: type_object];
                    let data_length = if data.is_null() {
                        0
                    } else {
                        msg_send![data, length]
                    };
                    let bytes = if data_length == 0 {
                        Vec::new()
                    } else {
                        let pointer: *const u8 = msg_send![data, bytes];
                        assert!(!pointer.is_null(), "pasteboard data should have bytes");
                        std::slice::from_raw_parts(pointer, data_length).to_vec()
                    };
                    types.push((type_name, bytes));
                }
                items.push(PasteboardItemSnapshot { types });
            }
            Self { items }
        }

        unsafe fn restore(&self) {
            let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
            if pasteboard.is_null() {
                return;
            }
            let _: isize = msg_send![pasteboard, clearContents];
            if self.items.is_empty() {
                return;
            }
            let objects: id =
                msg_send![class!(NSMutableArray), arrayWithCapacity: self.items.len()];
            if objects.is_null() {
                return;
            }
            for snapshot in &self.items {
                let allocated: id = msg_send![class!(NSPasteboardItem), alloc];
                let item: id = msg_send![allocated, init];
                if item.is_null() {
                    continue;
                }
                for (type_name, bytes) in &snapshot.types {
                    let type_c_string = CString::new(type_name.as_bytes())
                        .expect("pasteboard type should not contain nul");
                    let type_object: id = msg_send![
                        class!(NSString),
                        stringWithUTF8String: type_c_string.as_ptr()
                    ];
                    if type_object.is_null() {
                        continue;
                    }
                    let data: id = msg_send![
                        class!(NSData),
                        dataWithBytes: bytes.as_ptr()
                        length: bytes.len()
                    ];
                    let _: BOOL = msg_send![item, setData: data forType: type_object];
                }
                let _: () = msg_send![objects, addObject: item];
                let _: () = msg_send![item, release];
            }
            let _: BOOL = msg_send![pasteboard, writeObjects: objects];
        }
    }

    unsafe fn ns_string(value: id) -> Option<String> {
        if value.is_null() {
            return None;
        }
        let pointer: *const i8 = msg_send![value, UTF8String];
        if pointer.is_null() {
            return None;
        }
        CStr::from_ptr(pointer).to_str().ok().map(str::to_owned)
    }

    unsafe fn pasteboard_string() -> Option<String> {
        let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
        let type_name = CString::new("public.utf8-plain-text").expect("static type name");
        let type_object: id = msg_send![
            class!(NSString),
            stringWithUTF8String: type_name.as_ptr()
        ];
        let value: id = msg_send![pasteboard, stringForType: type_object];
        ns_string(value)
    }

    unsafe fn set_pasteboard_string(value: &str) {
        let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
        let type_name = CString::new("public.utf8-plain-text").expect("static type name");
        let type_object: id = msg_send![
            class!(NSString),
            stringWithUTF8String: type_name.as_ptr()
        ];
        let value = CString::new(value).expect("pasteboard text has no nul");
        let value_object: id = msg_send![
            class!(NSString),
            stringWithUTF8String: value.as_ptr()
        ];
        let _: isize = msg_send![pasteboard, clearContents];
        let _: BOOL = msg_send![pasteboard, setString: value_object forType: type_object];
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

    fn capture_frame(gui: &toybox::gpui_gui::GpuiHostedGui, context: &str) {
        let (width, height, pixels) = capture_pixels(gui, context);
        assert!(
            width > 0 && height > 0 && !pixels.is_empty(),
            "{context} should produce visible GPUI pixels"
        );
    }

    fn capture_pixels(gui: &toybox::gpui_gui::GpuiHostedGui, context: &str) -> (u32, u32, Vec<u8>) {
        let capture = gui
            .capture_rgba()
            .unwrap_or_else(|error| panic!("{context} should render a GPUI frame: {error}"));
        capture
    }

    unsafe fn send_mouse_event(window: id, event_type: usize, x: f64, top_y: f64, modifiers: u64) {
        let window_number: isize = msg_send![window, windowNumber];
        let location = NSPoint::new(x, f64::from(OUTPUT_HEIGHT) - top_y);
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

    unsafe fn send_mouse_move(window: id, x: f64, top_y: f64) {
        send_mouse_move_with_modifiers(window, x, top_y, 0);
    }

    unsafe fn send_mouse_move_with_modifiers(window: id, x: f64, top_y: f64, modifiers: u64) {
        send_mouse_event(window, 5, x, top_y, modifiers);
    }

    unsafe fn send_mouse_down(window: id, x: f64, top_y: f64, modifiers: u64) {
        send_mouse_event(window, 1, x, top_y, modifiers);
    }

    unsafe fn send_mouse_dragged(window: id, x: f64, top_y: f64, modifiers: u64) {
        send_mouse_event(window, 6, x, top_y, modifiers);
    }

    unsafe fn send_mouse_up(window: id, x: f64, top_y: f64, modifiers: u64) {
        send_mouse_event(window, 2, x, top_y, modifiers);
    }

    unsafe fn send_secondary_mouse_down(window: id, x: f64, top_y: f64, modifiers: u64) {
        send_mouse_event(window, 3, x, top_y, modifiers);
    }

    unsafe fn send_secondary_mouse_dragged(window: id, x: f64, top_y: f64, modifiers: u64) {
        send_mouse_event(window, 7, x, top_y, modifiers);
    }

    unsafe fn send_secondary_mouse_up(window: id, x: f64, top_y: f64, modifiers: u64) {
        send_mouse_event(window, 4, x, top_y, modifiers);
    }

    unsafe fn send_scroll_wheel(
        target_view: id,
        window: id,
        x: f64,
        top_y: f64,
        delta_y: f64,
        modifiers: u64,
    ) {
        let _ = modifiers;
        // AppKit has no public scroll-event constructor. Build a real
        // CoreGraphics event, wrap it as NSEvent, and deliver it to the
        // hosted NSView's native scrollWheel: entry point. Sending this local
        // event through NSApplication would require a global event post
        // because eventWithCGEvent: has no window number.
        let cg_event = CGEventCreateScrollWheelEvent2(
            std::ptr::null_mut(),
            1,
            1,
            delta_y.round() as i32,
            0,
            0,
        );
        assert!(
            !cg_event.is_null(),
            "CoreGraphics should create a scroll event"
        );
        let window_frame: NSRect = msg_send![window, frame];
        let screen: id = msg_send![window, screen];
        let screen_frame: NSRect = msg_send![screen, frame];
        let window_y = f64::from(OUTPUT_HEIGHT) - top_y;
        let appkit_global = NSPoint::new(
            screen_frame.origin.x + window_frame.origin.x + x,
            screen_frame.origin.y + window_frame.origin.y + window_y,
        );
        let quartz_location = NSPoint::new(
            appkit_global.x,
            screen_frame.origin.y + screen_frame.size.height - appkit_global.y,
        );
        CGEventSetLocation(cg_event, quartz_location);
        let event: id = msg_send![class!(NSEvent), eventWithCGEvent: cg_event];
        assert!(
            !event.is_null(),
            "AppKit should wrap the CoreGraphics scroll event"
        );
        let _: () = msg_send![target_view, scrollWheel: event];
        CFRelease(cg_event.cast_const());
    }

    unsafe fn send_click(window: id, x: f64, top_y: f64, modifiers: u64) {
        send_mouse_down(window, x, top_y, modifiers);
        send_mouse_up(window, x, top_y, modifiers);
    }

    unsafe fn send_key(
        app: id,
        window: id,
        gui: &toybox::gpui_gui::GpuiHostedGui,
        text: &str,
        key_code: u16,
        modifiers: u64,
    ) {
        send_key_with_repeat(app, window, gui, text, key_code, modifiers, false);
    }

    unsafe fn send_repeated_key(
        app: id,
        window: id,
        gui: &toybox::gpui_gui::GpuiHostedGui,
        text: &str,
        key_code: u16,
        modifiers: u64,
    ) {
        send_key_with_repeat(app, window, gui, text, key_code, modifiers, true);
    }

    unsafe fn send_key_with_repeat(
        app: id,
        window: id,
        gui: &toybox::gpui_gui::GpuiHostedGui,
        text: &str,
        key_code: u16,
        modifiers: u64,
        repeat: bool,
    ) {
        let bytes = CString::new(text).expect("key text has no nul");
        let characters: id = msg_send![
            class!(NSString),
            stringWithUTF8String: bytes.as_ptr()
        ];
        let window_number: isize = msg_send![window, windowNumber];
        let down: id = msg_send![
            class!(NSEvent),
            keyEventWithType: 10_usize
            location: NSPoint::new(48.0, 28.0)
            modifierFlags: modifiers
            timestamp: 0.0_f64
            windowNumber: window_number
            context: std::ptr::null_mut::<Object>()
            characters: characters
            charactersIgnoringModifiers: characters
            isARepeat: NO
            keyCode: key_code
        ];
        let _: () = msg_send![window, sendEvent: down];
        if repeat {
            let repeated_down: id = msg_send![
                class!(NSEvent),
                keyEventWithType: 10_usize
                location: NSPoint::new(48.0, 28.0)
                modifierFlags: modifiers
                timestamp: 0.0_f64
                windowNumber: window_number
                context: std::ptr::null_mut::<Object>()
                characters: characters
                charactersIgnoringModifiers: characters
                isARepeat: YES
                keyCode: key_code
            ];
            let _: () = msg_send![window, sendEvent: repeated_down];
        }
        let up: id = msg_send![
            class!(NSEvent),
            keyEventWithType: 11_usize
            location: NSPoint::new(48.0, 28.0)
            modifierFlags: modifiers
            timestamp: 0.0_f64
            windowNumber: window_number
            context: std::ptr::null_mut::<Object>()
            characters: characters
            charactersIgnoringModifiers: characters
            isARepeat: NO
            keyCode: key_code
        ];
        let _: () = msg_send![window, sendEvent: up];
        pump_appkit(app, gui, 0.02);
    }

    fn mac_key_code(character: char) -> u16 {
        match character {
            '0' => 29,
            '1' => 18,
            '2' => 19,
            '3' => 20,
            '4' => 21,
            '5' => 23,
            '6' => 22,
            '7' => 26,
            '8' => 28,
            '9' => 25,
            '.' => 47,
            '-' => 27,
            'x' => 7,
            ' ' => 49,
            'm' => 46,
            's' => 1,
            _ => panic!("native input fixture has no key code for {character:?}"),
        }
    }

    unsafe fn send_text(app: id, window: id, gui: &toybox::gpui_gui::GpuiHostedGui, text: &str) {
        for character in text.chars() {
            let character_text = character.to_string();
            send_key(
                app,
                window,
                gui,
                &character_text,
                mac_key_code(character),
                0,
            );
        }
    }

    unsafe fn assert_delay_text_edit(
        app: id,
        fixture: &NativeFixture,
        gui: &toybox::gpui_gui::GpuiHostedGui,
        text: &str,
    ) {
        send_click(fixture.window, DELAY_X, DELAY_Y, 0);
        pump_appkit(app, gui, 0.03);
        send_key(app, fixture.window, gui, "a", 0, COMMAND);
        send_text(app, fixture.window, gui, text);
        send_key(app, fixture.window, gui, "\r", 36, 0);
    }

    unsafe fn exercise_native_input(fixture: &NativeFixture) {
        let app = NSApp();
        let (mut gui, params, _status) = new_screenshot_gui_with_params();
        gui.set_parent_raw(fixture.parent_handle());
        assert!(gui.open(), "Pump GPUI input fixture should open");
        gui.request_resize(OUTPUT_WIDTH, OUTPUT_HEIGHT);
        let _: () = msg_send![
            fixture.window,
            makeFirstResponder: std::ptr::null_mut::<Object>()
        ];
        pump_appkit(app, &gui, 0.1);
        capture_frame(&gui, "opened editor");

        let initial_sync_division = params.sync_division();
        let initial_bypass = params.bypassed();

        // With no focused GPUI control, Space remains available to the host.
        send_repeated_key(app, fixture.window, &gui, " ", 49, 0);
        assert_eq!(
            params.bypassed(),
            initial_bypass,
            "unfocused transport Space must not toggle bypass"
        );

        // The entire delay control, including its progress strip and padding,
        // owns numeric focus after the Sync menu has been used.
        for y in [22.0, 31.0, DELAY_Y] {
            send_click(fixture.window, TIMING_VALUE_X, TIMING_VALUE_Y, 0);
            pump_appkit(app, &gui, 0.04);
            let sync = params.sync_division();
            let delay = params.delay_beats();
            send_click(fixture.window, DELAY_X, y, 0);
            pump_appkit(app, &gui, 0.04);
            send_key(app, fixture.window, &gui, "\u{f700}", 126, 0);
            assert_eq!(
                params.sync_division(),
                sync,
                "delay field at y={y} must own Up instead of Sync"
            );
            assert_eq!(
                params.delay_beats(),
                delay + 1,
                "delay field at y={y} must step delay"
            );
            send_key(app, fixture.window, &gui, "\u{f701}", 125, 0);
            send_key(app, fixture.window, &gui, "\r", 36, 0);
        }

        // The timing value opens the native GPUI menu. Selecting another
        // sync subdivision must update the shared parameter and close it.
        let selected_sync_division = if initial_sync_division == 6 { 5 } else { 6 };
        send_click(fixture.window, TIMING_VALUE_X, TIMING_VALUE_Y, 0);
        pump_appkit(app, &gui, 0.04);
        capture_frame(&gui, "opened timing dropdown");
        send_click(
            fixture.window,
            TIMING_VALUE_X,
            TIMING_OPTION_FIRST_CENTER_Y + TIMING_OPTION_STEP_Y * selected_sync_division as f64,
            0,
        );
        pump_appkit(app, &gui, 0.04);
        assert_eq!(
            params.sync_division(),
            selected_sync_division,
            "native timing dropdown should select a sync subdivision"
        );

        // Filter handles are drawn above the curve canvas and must capture a
        // real native drag before the curve's own gesture admission runs.
        // Use the minimum Q guide edge for a stable 640x400 hit target.
        params.set_filter_enabled(1.0);
        params.set_filter_hp_freq_hz(320.0);
        params.set_filter_hp_q(0.25);
        params.set_filter_lp_freq_hz(4_800.0);
        params.set_filter_lp_q(0.25);
        pump_appkit(app, &gui, 0.05);
        let filter_curve_before = params.editable_curve_snapshot();
        let filter_hp_before = params.filter_hp_freq_hz();
        send_mouse_move(fixture.window, FILTER_HP_HANDLE_X, FILTER_HANDLE_MIN_Q_Y);
        pump_appkit(app, &gui, 0.04);
        send_mouse_down(fixture.window, FILTER_HP_HANDLE_X, FILTER_HANDLE_MIN_Q_Y, 0);
        send_mouse_dragged(
            fixture.window,
            FILTER_HP_HANDLE_X + 42.0,
            FILTER_HANDLE_MIN_Q_Y - 10.0,
            0,
        );
        send_mouse_up(
            fixture.window,
            FILTER_HP_HANDLE_X + 42.0,
            FILTER_HANDLE_MIN_Q_Y - 10.0,
            0,
        );
        pump_appkit(app, &gui, 0.05);
        assert!(
            params.filter_hp_freq_hz() > filter_hp_before,
            "native HP handle drag should update its cutoff"
        );
        assert_eq!(
            params.filter_lp_freq_hz(),
            4_800.0,
            "native HP handle drag must leave LP cutoff unchanged"
        );
        assert_eq!(
            params.editable_curve_snapshot(),
            filter_curve_before,
            "native filter handle drag must not mutate the pump curve"
        );
        params.set_filter_enabled(0.0);

        // Exercise the curve's retained visual feedback through real native
        // hover/modifier/drag events before mutating its authored points.
        let curve_before_feedback = params.editable_curve_snapshot();
        pump_appkit(app, &gui, 0.04);
        let curve_idle_capture = capture_pixels(&gui, "idle curve feedback");
        send_mouse_move(fixture.window, CURVE_NODE_X, CURVE_NODE_Y);
        pump_appkit(app, &gui, 0.04);
        let node_hover_capture = capture_pixels(&gui, "node hover feedback");
        assert!(
            curve_idle_capture.2 != node_hover_capture.2,
            "native node hover should repaint the curve"
        );
        send_mouse_move(fixture.window, CURVE_SEGMENT_X, CURVE_SEGMENT_Y);
        pump_appkit(app, &gui, 0.04);
        let segment_hover_capture = capture_pixels(&gui, "segment proximity feedback");
        let blue_pixels = segment_hover_capture
            .2
            .chunks_exact(4)
            .filter(|rgba| {
                rgba[2] > 180
                    && rgba[2].saturating_sub(rgba[0]) > 70
                    && rgba[1].saturating_sub(rgba[0]) > 30
            })
            .count();
        assert!(blue_pixels > 50, "segment hover must contain blue feedback");
        assert!(
            node_hover_capture.2 != segment_hover_capture.2,
            "native segment proximity hover should repaint the curve"
        );
        send_mouse_move_with_modifiers(
            fixture.window,
            CURVE_SEGMENT_X,
            CURVE_SEGMENT_Y + 11.0,
            COMMAND,
        );
        pump_appkit(app, &gui, 0.04);
        let command_hover_capture = capture_pixels(&gui, "command segment feedback");
        assert!(
            command_hover_capture
                .2
                .chunks_exact(4)
                .filter(|rgba| {
                    rgba[2] > 180
                        && rgba[2].saturating_sub(rgba[0]) > 70
                        && rgba[1].saturating_sub(rgba[0]) > 30
                })
                .count()
                > 50,
            "Command segment hover must paint the blue move overlay"
        );

        let endpoint_count = curve_before_feedback.nodes.len();
        send_mouse_down(fixture.window, CURVE_ENDPOINT_X, CURVE_ENDPOINT_Y, OPTION);
        send_mouse_up(fixture.window, CURVE_ENDPOINT_X, CURVE_ENDPOINT_Y, OPTION);
        pump_appkit(app, &gui, 0.04);
        assert_eq!(
            params.editable_curve_snapshot().nodes.len(),
            endpoint_count,
            "Option-click must protect curve endpoints"
        );
        send_mouse_down(fixture.window, 222.0, 143.0, OPTION);
        send_mouse_up(fixture.window, 222.0, 143.0, OPTION);
        pump_appkit(app, &gui, 0.04);
        assert_eq!(
            params.editable_curve_snapshot().nodes.len(),
            endpoint_count.saturating_sub(1),
            "Option-click should delete an interior curve node"
        );

        let curve_before_left_blank = params.editable_curve_snapshot();
        send_click(fixture.window, CURVE_PLOT_X, CURVE_PLOT_Y, 0);
        pump_appkit(app, &gui, 0.04);
        assert_eq!(
            params.editable_curve_snapshot(),
            curve_before_left_blank,
            "a blank click without dragging must not insert a node"
        );
        send_mouse_down(fixture.window, CURVE_PLOT_X, CURVE_PLOT_Y, 0);
        send_mouse_dragged(fixture.window, CURVE_PLOT_X + 40.0, 200.0, 0);
        send_mouse_up(fixture.window, CURVE_PLOT_X + 40.0, 200.0, 0);
        pump_appkit(app, &gui, 0.04);
        assert_eq!(
            params.editable_curve_snapshot().nodes.len(),
            curve_before_left_blank.nodes.len() + 1,
            "left drag on empty curve space must add exactly one node"
        );
        pump_appkit(app, &gui, 0.04);
        let curve_before_right_paint = params.editable_curve_snapshot();
        let paint_idle_capture = capture_pixels(&gui, "idle right paint");
        send_secondary_mouse_down(fixture.window, CURVE_NODE_X, CURVE_NODE_Y, 0);
        send_secondary_mouse_dragged(fixture.window, CURVE_PLOT_X, CURVE_PLOT_Y, 0);
        pump_appkit(app, &gui, 0.04);
        let paint_preview_capture = capture_pixels(&gui, "right paint preview");
        assert!(
            paint_idle_capture.2 != paint_preview_capture.2,
            "right drag should show a live paint preview"
        );
        send_secondary_mouse_up(fixture.window, CURVE_PLOT_X, CURVE_PLOT_Y, 0);
        pump_appkit(app, &gui, 0.04);
        assert_ne!(
            params.editable_curve_snapshot(),
            curve_before_right_paint,
            "right drag should commit freehand painting"
        );

        let phase_before_offset = params.phase_offset();
        send_mouse_down(fixture.window, 58.0, 245.0, COMMAND | SHIFT);
        pump_appkit(app, &gui, 0.04);
        let offset_active_capture = capture_pixels(&gui, "active curve offset");
        send_mouse_dragged(fixture.window, 105.0, 245.0, COMMAND | SHIFT);
        pump_appkit(app, &gui, 0.04);
        let offset_sliding_capture = capture_pixels(&gui, "sliding curve offset");
        assert!(
            offset_active_capture.2 != offset_sliding_capture.2,
            "offset drag should repaint the widened yellow curve"
        );
        send_mouse_up(fixture.window, 105.0, 245.0, COMMAND | SHIFT);
        pump_appkit(app, &gui, 0.04);
        let visible_shift = (phase_before_offset - params.phase_offset()).rem_euclid(1.0);
        assert!(
            visible_shift > 0.05 && visible_shift < 0.15,
            "rightward Cmd+Shift drag must shift the curve right, got {visible_shift}"
        );

        send_mouse_down(fixture.window, 300.0, 100.0, SHIFT);
        send_mouse_dragged(fixture.window, 400.0, 210.0, SHIFT);
        pump_appkit(app, &gui, 0.04);
        let marquee_capture = capture_pixels(&gui, "marquee feedback");
        assert!(
            offset_sliding_capture.2 != marquee_capture.2,
            "Shift drag should repaint the marquee"
        );
        send_mouse_up(fixture.window, 400.0, 210.0, SHIFT);
        send_mouse_move(fixture.window, -20.0, 100.0);
        pump_appkit(app, &gui, 0.04);
        let curve_leave_capture = capture_pixels(&gui, "curve leave feedback reset");
        assert!(
            marquee_capture.2 != curve_leave_capture.2,
            "native curve leave/release should clear transient feedback"
        );

        // Offset changes only project the seam. Both edge handles edit one
        // authored point, and horizontal pointer motion cannot move that point.
        let saved_curve = params.editable_curve_snapshot();
        let saved_phase = params.phase_offset();
        let mut flat_curve = curve_before_feedback.clone();
        for node in &mut flat_curve.nodes {
            node.y = 0.5;
        }
        for segment in &mut flat_curve.segments {
            segment.tension = 0.0;
        }
        for edge_x in [53.0, 583.0] {
            params.set_editable_curve(&flat_curve);
            params.set_phase_offset(0.25);
            pump_appkit(app, &gui, 0.05);
            send_click(fixture.window, edge_x, 147.0, 0);
            pump_appkit(app, &gui, 0.04);
            assert_eq!(
                params.editable_curve_snapshot(),
                flat_curve,
                "clicking a virtual seam without dragging must not insert a node"
            );
            send_mouse_down(fixture.window, edge_x, 147.0, 0);
            send_mouse_dragged(fixture.window, edge_x, 120.0, 0);
            send_mouse_dragged(fixture.window, edge_x + 30.0, 110.0, 0);
            send_mouse_up(fixture.window, edge_x + 30.0, 110.0, 0);
            pump_appkit(app, &gui, 0.04);
            let edited = params.editable_curve_snapshot();
            assert_eq!(
                edited.nodes.len(),
                flat_curve.nodes.len() + 1,
                "either seam copy must materialize exactly one node"
            );
            let seam = edited
                .nodes
                .iter()
                .find(|node| (node.x - 0.25).abs() < 0.00001)
                .expect("dragged seam must stay at the viewport boundary's authored phase");
            assert!(
                seam.y > 0.65 && seam.y < 0.8,
                "seam must follow vertical drag: {seam:?}"
            );
        }
        for (source_x, edge_x) in [(90.0, 52.0), (493.0, 584.0)] {
            params.set_editable_curve(&flat_curve);
            params.set_phase_offset(0.25);
            pump_appkit(app, &gui, 0.04);
            send_mouse_down(fixture.window, source_x, 147.0, 0);
            send_mouse_dragged(fixture.window, edge_x, 120.0, 0);
            send_mouse_up(fixture.window, edge_x, 120.0, 0);
            pump_appkit(app, &gui, 0.04);
            let merged = params.editable_curve_snapshot();
            assert_eq!(
                merged
                    .nodes
                    .iter()
                    .filter(|node| (node.x - 0.25).abs() < 0.00001)
                    .count(),
                1,
                "source {source_x} to edge {edge_x} must become seam: {merged:?}"
            );
            assert_eq!(
                merged.nodes.len(),
                flat_curve.nodes.len(),
                "seam takeover must move/merge the source rather than add another node"
            );
        }
        // The authored cycle-zero point is an ordinary movable point when
        // offset places it inside the viewport. Closure anchors are hidden.
        for delta in [-40.0, 20.0] {
            params.set_editable_curve(&flat_curve);
            params.set_phase_offset(0.25);
            pump_appkit(app, &gui, 0.04);
            send_mouse_down(fixture.window, 451.0, 147.0, 0);
            send_mouse_dragged(fixture.window, 451.0 + delta, 120.0, 0);
            send_mouse_up(fixture.window, 451.0 + delta, 120.0, 0);
            pump_appkit(app, &gui, 0.04);
            let moved = params.editable_curve_snapshot();
            let expected_x = (delta as f32 / 530.0).rem_euclid(1.0);
            assert!(
                moved.origin_is_clip,
                "moving cycle zero must replace it with hidden closure anchors"
            );
            assert!(
                moved
                    .nodes
                    .iter()
                    .skip(1)
                    .take(moved.nodes.len() - 2)
                    .any(|node| (node.x - expected_x).abs() < 0.004 && node.y > 0.6),
                "cycle-zero point must follow the horizontal drag: {moved:?}"
            );
            assert_eq!(
                params.phase_offset(),
                0.25,
                "point dragging must not change offset"
            );
            // A second gesture must cross authored zero without deleting the
            // point or swallowing unrelated nodes on the other side of it.
            send_mouse_down(fixture.window, 451.0 + delta, 120.0, 0);
            send_mouse_dragged(fixture.window, 451.0 - delta, 110.0, 0);
            send_mouse_up(fixture.window, 451.0 - delta, 110.0, 0);
            pump_appkit(app, &gui, 0.04);
            let crossed = params.editable_curve_snapshot();
            let crossed_x = (-delta as f32 / 530.0).rem_euclid(1.0);
            assert_eq!(crossed.nodes.len(), moved.nodes.len());
            assert!(
                crossed
                    .nodes
                    .iter()
                    .any(|node| { (node.x - crossed_x).abs() < 0.004 && node.y > 0.65 }),
                "second drag must cross authored zero freely: {crossed:?}"
            );
        }

        // Hosts may send Backspace as a virtual key or a character-only
        // callback. All forms must edit the focused draft and be consumed.
        for (character, code) in [(0, 1), (127, 0), (8, 0)] {
            send_click(fixture.window, DELAY_X, DELAY_Y, 0);
            send_key(app, fixture.window, &gui, "a", 0, COMMAND);
            send_text(app, fixture.window, &gui, "12");
            assert!(
                gui.on_key_down(character, code, 0),
                "focused Backspace must be consumed"
            );
            gui.on_key_up(character, code, 0);
            pump_appkit(app, &gui, 0.04);
            send_key(app, fixture.window, &gui, "\r", 36, 0);
            assert_eq!(
                params.delay_beats(),
                1,
                "Backspace form ({character}, {code}) must remove the last digit"
            );
        }

        // Marquee selection takes keyboard ownership from an earlier numeric
        // field, and Backspace deletes the selection rather than its text.
        assert_delay_text_edit(app, fixture, &gui, "4");
        params.set_editable_curve(&curve_before_feedback);
        params.set_phase_offset(0.0);
        pump_appkit(app, &gui, 0.04);
        send_mouse_down(fixture.window, 80.0, 130.0, SHIFT);
        send_mouse_dragged(fixture.window, 240.0, 225.0, SHIFT);
        send_mouse_up(fixture.window, 240.0, 225.0, SHIFT);
        pump_appkit(app, &gui, 0.04);
        send_key(app, fixture.window, &gui, "\u{7f}", 51, 0);
        assert_eq!(
            params.editable_curve_snapshot().nodes.len(),
            2,
            "Backspace must delete marquee-selected interior nodes"
        );
        assert_eq!(
            params.delay_beats(),
            4,
            "curve deletion must not edit delay"
        );
        params.set_editable_curve(&saved_curve);
        params.set_phase_offset(saved_phase);
        pump_appkit(app, &gui, 0.04);

        // The delay field is reached by a native click, and its text is
        // inserted through AppKit's interpretKeyEvents path.
        assert_delay_text_edit(app, fixture, &gui, "7");
        assert_eq!(params.delay_beats(), 7, "native delay text should commit 7");

        // Delay drafts admit digits only. Invalid text and invalid paste are
        // rejected atomically, while Backspace can clear the active draft so
        // a new integer can be entered.
        send_click(fixture.window, DELAY_X, DELAY_Y, 0);
        send_key(app, fixture.window, &gui, "a", 0, COMMAND);
        send_text(app, fixture.window, &gui, "x");
        send_key(app, fixture.window, &gui, "\r", 36, 0);
        assert_eq!(
            params.delay_beats(),
            7,
            "invalid Delay text must be rejected without changing the value"
        );
        let invalid_delay_pasteboard_restore = PasteboardRestore::capture();
        set_pasteboard_string("not beats");
        send_click(fixture.window, DELAY_X, DELAY_Y, 0);
        send_key(app, fixture.window, &gui, "a", 0, COMMAND);
        send_key(app, fixture.window, &gui, "v", 9, COMMAND);
        send_key(app, fixture.window, &gui, "\r", 36, 0);
        assert_eq!(
            params.delay_beats(),
            7,
            "invalid Delay paste must be rejected without changing the value"
        );
        drop(invalid_delay_pasteboard_restore);
        send_click(fixture.window, DELAY_X, DELAY_Y, 0);
        send_key(app, fixture.window, &gui, "\u{8}", 51, 0);
        send_text(app, fixture.window, &gui, "7");
        send_key(app, fixture.window, &gui, "\r", 36, 0);
        assert_eq!(
            params.delay_beats(),
            7,
            "Delay Backspace clear and retype should commit the replacement integer"
        );

        let sync_before_active_delay_arrow = params.sync_division();
        send_click(fixture.window, DELAY_X, DELAY_Y, 0);
        send_key(app, fixture.window, &gui, "\u{f700}", 126, 0);
        send_key(app, fixture.window, &gui, "\u{f701}", 125, SHIFT);
        assert_eq!(
            params.delay_beats(),
            4,
            "active Delay Up then Shift+Down should update the delay"
        );
        assert_eq!(
            params.sync_division(),
            sync_before_active_delay_arrow,
            "active Delay arrows must not change sync division"
        );
        send_key(app, fixture.window, &gui, "\r", 36, 0);

        send_key(app, fixture.window, &gui, "\u{f700}", 126, 0);
        send_key(app, fixture.window, &gui, "\u{f701}", 125, SHIFT);
        assert_eq!(
            params.delay_beats(),
            1,
            "focused inactive Up then Shift+Down should step the delay"
        );
        assert_eq!(
            params.sync_division(),
            sync_before_active_delay_arrow,
            "delay arrows must not change sync division"
        );
        assert_delay_text_edit(app, fixture, &gui, "4");

        // Exercise selection plus the real native pasteboard, then restore the
        // committed value after cut/paste and after an invalid Escape draft.
        let delay_pasteboard_restore = PasteboardRestore::capture();
        send_key(app, fixture.window, &gui, "a", 0, COMMAND);
        send_key(app, fixture.window, &gui, "c", 8, COMMAND);
        assert_eq!(
            pasteboard_string(),
            Some("4 beats".to_owned()),
            "copy should write the selected delay text"
        );
        send_key(app, fixture.window, &gui, "x", 7, COMMAND);
        send_key(app, fixture.window, &gui, "v", 9, COMMAND);
        send_key(app, fixture.window, &gui, "\r", 36, 0);
        assert_eq!(
            params.delay_beats(),
            4,
            "cut then paste should restore delay 4"
        );

        send_key(app, fixture.window, &gui, "\u{f729}", 115, 0);
        send_key(app, fixture.window, &gui, "\u{f703}", 124, SHIFT);
        send_key(app, fixture.window, &gui, "c", 8, COMMAND);
        assert_eq!(
            pasteboard_string(),
            Some("4".to_owned()),
            "selection should copy one delay grapheme"
        );

        send_key(app, fixture.window, &gui, "a", 0, COMMAND);
        send_text(app, fixture.window, &gui, "x");
        send_key(app, fixture.window, &gui, "\u{1b}", 53, 0);
        assert_eq!(
            params.delay_beats(),
            4,
            "Escape should restore committed delay"
        );
        drop(delay_pasteboard_restore);
        send_click(fixture.window, DELAY_X, DELAY_Y, 0);
        send_key(app, fixture.window, &gui, "a", 0, COMMAND);
        send_key(app, fixture.window, &gui, "\u{8}", 51, 0);
        send_key(app, fixture.window, &gui, "\r", 36, 0);
        assert_eq!(params.delay_beats(), 4, "empty Enter reverts the delay");

        // A plain drag on the Smooth knob changes its parameter through the
        // native pointer path. Command-clicking the value enters the numeric
        // field, which then receives text from native NSEvents.
        let initial_smooth = params.smooth();
        send_mouse_move(fixture.window, SMOOTH_X, SMOOTH_KNOB_Y);
        send_mouse_down(fixture.window, SMOOTH_X, SMOOTH_KNOB_Y, 0);
        send_mouse_dragged(fixture.window, SMOOTH_X, SMOOTH_KNOB_Y, 0);
        send_mouse_dragged(fixture.window, SMOOTH_X, 300.0, 0);
        let before_crossing = params.smooth();
        // Cross the curve and then leave the native window before releasing.
        // The admitted knob gesture must retain ownership throughout.
        send_mouse_dragged(fixture.window, 300.0, 150.0, 0);
        assert!(
            params.smooth() > before_crossing,
            "knob must continue over the curve"
        );
        let before_leaving = params.smooth();
        send_mouse_dragged(fixture.window, 650.0, 180.0, 0);
        assert!(
            (params.smooth() - (before_leaving - 0.12)).abs() < 0.001,
            "knob must continue outside the right edge"
        );
        send_mouse_dragged(fixture.window, -20.0, 200.0, 0);
        assert!(
            (params.smooth() - (before_leaving - 0.20)).abs() < 0.001,
            "knob must continue outside the left edge"
        );
        send_mouse_up(fixture.window, -20.0, 200.0, 0);
        let after_release = params.smooth();
        send_mouse_move(fixture.window, 300.0, 120.0);
        assert_eq!(
            params.smooth(),
            after_release,
            "release ends drag ownership"
        );
        pump_appkit(app, &gui, 0.05);
        assert!(
            params.smooth() > initial_smooth,
            "plain Smooth knob drag should change the parameter"
        );

        // A plain value click selects the control without admitting text
        // editing. Native text and Enter must leave the parameter unchanged;
        // Command-click below is the explicit numeric-edit admission path.
        let smooth_before_plain_value_click = params.smooth();
        send_click(fixture.window, SMOOTH_X, SMOOTH_VALUE_Y, 0);
        send_text(app, fixture.window, &gui, "9");
        send_key(app, fixture.window, &gui, "\r", 36, 0);
        assert_eq!(
            params.smooth(),
            smooth_before_plain_value_click,
            "plain Smooth value click must not start numeric text editing"
        );

        send_click(fixture.window, SMOOTH_X, SMOOTH_VALUE_Y, COMMAND);
        pump_appkit(app, &gui, 0.03);
        send_key(app, fixture.window, &gui, "a", 0, COMMAND);
        send_text(app, fixture.window, &gui, "25");
        send_key(app, fixture.window, &gui, "\r", 36, 0);
        assert!(
            (params.smooth() - 0.25).abs() < 1.0e-4,
            "Command-click Smooth value should enter numeric editing (got {})",
            params.smooth()
        );

        // Wheel and arrow input on the knob itself must update the parameter
        // through discrete knob transactions without entering text editing.
        let smooth_before_wheel = params.smooth();
        send_click(fixture.window, SMOOTH_X, SMOOTH_KNOB_Y, 0);
        pump_appkit(app, &gui, 0.03);
        send_scroll_wheel(
            fixture.hosted_view(),
            fixture.window,
            SMOOTH_X,
            SMOOTH_KNOB_Y,
            12.0,
            0,
        );
        pump_appkit(app, &gui, 0.04);
        assert_ne!(
            params.smooth(),
            smooth_before_wheel,
            "native Smooth wheel should update the parameter"
        );
        let smooth_before_arrow = params.smooth();
        send_key(app, fixture.window, &gui, "\u{f700}", 126, 0);
        assert_ne!(
            params.smooth(),
            smooth_before_arrow,
            "native Smooth arrow should update the parameter without text editing"
        );

        // Free mode exposes RATE between Swing and Mix. Select milliseconds
        // and edit its real native numeric field.
        send_click(fixture.window, TIMING_MODE_X, TIMING_VALUE_Y, 0);
        pump_appkit(app, &gui, 0.04);
        assert_eq!(
            params.timing_mode(),
            1,
            "native timing mode button should enter free-rate mode"
        );
        send_click(fixture.window, TIMING_VALUE_X, TIMING_VALUE_Y, 0);
        capture_frame(&gui, "opened free-rate unit dropdown");
        send_click(
            fixture.window,
            TIMING_VALUE_X,
            TIMING_OPTION_FIRST_CENTER_Y,
            0,
        );
        pump_appkit(app, &gui, 0.04);
        let free_rate_pasteboard_restore = PasteboardRestore::capture();
        send_click(fixture.window, DELAY_X, DELAY_Y, 0);
        send_key(app, fixture.window, &gui, "a", 0, COMMAND);
        send_key(app, fixture.window, &gui, "c", 8, COMMAND);
        assert_eq!(
            pasteboard_string(),
            Some("4".to_owned()),
            "free-rate unit selection should close its menu before the native delay field is reachable"
        );
        send_key(app, fixture.window, &gui, "\u{1b}", 53, 0);
        drop(free_rate_pasteboard_restore);
        // Start from a non-default RATE so this assertion proves the native
        // edit changes the parameter rather than merely observing its setup.
        params.set_free_rate_hz(4.0);
        pump_appkit(app, &gui, 0.04);
        send_click(fixture.window, 320.0, SMOOTH_VALUE_Y, COMMAND);
        pump_appkit(app, &gui, 0.03);
        send_key(app, fixture.window, &gui, "a", 0, COMMAND);
        send_text(app, fixture.window, &gui, "500 ms");
        send_key(app, fixture.window, &gui, "\r", 36, 0);
        assert!(
            (params.free_rate_hz() - 2.0).abs() < 1.0e-4,
            "native RATE entry with explicit milliseconds should set 500 ms (2 Hz), got {} Hz",
            params.free_rate_hz()
        );

        // Return to sync timing before the bypass and reopen checks, which
        // continue to use the delay field.
        send_click(fixture.window, TIMING_MODE_X, TIMING_VALUE_Y, 0);
        pump_appkit(app, &gui, 0.04);
        assert_eq!(
            params.timing_mode(),
            0,
            "native timing mode button should return to sync mode"
        );

        // Apply a plain host parameter event while the editor is idle. The
        // assertion reads the rendered native field through its clipboard
        // path, so it verifies host projection rather than the shared atomics
        // alone.
        params.set_smooth(0.61);
        pump_appkit(app, &gui, 0.12);
        let idle_pasteboard_restore = PasteboardRestore::capture();
        send_click(fixture.window, SMOOTH_X, SMOOTH_VALUE_Y, COMMAND);
        send_key(app, fixture.window, &gui, "a", 0, COMMAND);
        send_key(app, fixture.window, &gui, "c", 8, COMMAND);
        assert_eq!(
            pasteboard_string(),
            Some("61%".to_owned()),
            "idle host Smooth update should reach the visible native field"
        );
        send_key(app, fixture.window, &gui, "\u{1b}", 53, 0);
        drop(idle_pasteboard_restore);

        // Focus the native bypass button, then verify repeated Space and
        // Enter each produce one activation.
        send_click(fixture.window, BYPASS_X, BYPASS_Y, 0);
        pump_appkit(app, &gui, 0.03);
        assert!(
            !initial_bypass,
            "fixture should begin active (not bypassed)"
        );
        assert!(params.bypassed(), "native bypass click should toggle once");
        let bypass_after_click = params.bypassed();
        send_repeated_key(app, fixture.window, &gui, " ", 49, 0);
        assert_eq!(
            params.bypassed(),
            !bypass_after_click,
            "repeated Space should toggle bypass exactly once"
        );
        send_key(app, fixture.window, &gui, "\r", 36, 0);
        assert_eq!(
            params.bypassed(),
            bypass_after_click,
            "Enter should toggle bypass exactly once"
        );

        gui.close();
        gui.set_parent_raw(fixture.parent_handle());
        assert!(gui.open(), "Pump GPUI input fixture should reopen");
        gui.request_resize(OUTPUT_WIDTH, OUTPUT_HEIGHT);
        let _: () = msg_send![
            fixture.window,
            makeFirstResponder: std::ptr::null_mut::<Object>()
        ];
        pump_appkit(app, &gui, 0.1);
        capture_frame(&gui, "reopened editor");
        assert_delay_text_edit(app, fixture, &gui, "6");
        assert_eq!(
            params.delay_beats(),
            6,
            "native delay input should still work after close and reopen"
        );
        gui.close();
        eprintln!(
            "PASS native Pump GPUI filter handle capture/drag, delay typing/arrows/Backspace, marquee deletion, cyclic node drags, seam handles, insertion, offset direction, timing dropdown, clipboard, Smooth controls, host projection, transport, and reopen input"
        );
    }

    pub fn run() {
        unsafe {
            let pool = NSAutoreleasePool::new(nil);
            let _curve_slot_sandbox = CurveSlotSandbox::new();
            let fixture = NativeFixture::new();
            exercise_native_input(&fixture);
            drop(fixture);
            pool.drain();
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Pump GPUI input fixture requires macOS");
    std::process::exit(1);
}
