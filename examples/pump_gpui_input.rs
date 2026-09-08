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
        let (width, height, pixels) = gui
            .capture_rgba()
            .unwrap_or_else(|error| panic!("{context} should render a GPUI frame: {error}"));
        assert!(
            width > 0 && height > 0 && !pixels.is_empty(),
            "{context} should produce visible GPUI pixels"
        );
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
        let _: () = msg_send![window, sendEvent: event];
    }

    unsafe fn send_mouse_move(window: id, x: f64, top_y: f64) {
        send_mouse_event(window, 5, x, top_y, 0);
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

        // The timing value opens the native GPUI menu. Selecting another
        // sync subdivision must update the shared parameter and close it.
        let selected_sync_division = if initial_sync_division == 6 { 5 } else { 6 };
        send_click(fixture.window, TIMING_VALUE_X, TIMING_VALUE_Y, 0);
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

        // The delay field is reached by a native click, and its text is
        // inserted through AppKit's interpretKeyEvents path.
        assert_delay_text_edit(app, fixture, &gui, "7");
        assert_eq!(params.delay_beats(), 7, "native delay text should commit 7");
        send_key(app, fixture.window, &gui, "\u{f700}", 126, 0);
        send_key(app, fixture.window, &gui, "\u{f701}", 125, SHIFT);
        assert_eq!(params.delay_beats(), 4, "Up then Shift+Down should yield 4");
        assert_eq!(
            params.sync_division(),
            selected_sync_division,
            "delay arrows must not change sync division"
        );

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

        // A plain drag on the Smooth knob changes its parameter through the
        // native pointer path. Command-clicking the value enters the numeric
        // field, which then receives text from native NSEvents.
        let initial_smooth = params.smooth();
        send_mouse_move(fixture.window, SMOOTH_X, SMOOTH_KNOB_Y);
        send_mouse_down(fixture.window, SMOOTH_X, SMOOTH_KNOB_Y, 0);
        send_mouse_dragged(fixture.window, SMOOTH_X, SMOOTH_KNOB_Y, 0);
        send_mouse_dragged(fixture.window, SMOOTH_X, 300.0, 0);
        send_mouse_up(fixture.window, SMOOTH_X, 300.0, 0);
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
            Some("4 beats".to_owned()),
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
            "PASS native Pump GPUI timing dropdown, delay typing/arrows, clipboard selection/cut/paste, Escape, Smooth drag/edit/wheel/arrows, idle host projection, bypass Space/Enter, unfocused transport, and reopen input"
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
