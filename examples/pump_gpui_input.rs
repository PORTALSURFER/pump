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
            'z' => 6,
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

    fn center(key: &str) -> (f64, f64) {
        let [x, y, w, h] = pump::gui_gpui::screenshot_bounds(key);
        (f64::from(x + w * 0.5), f64::from(y + h * 0.5))
    }

    unsafe fn click_control(fixture: &NativeFixture, key: &str, modifiers: u64) {
        let (x, y) = center(key);
        send_click(fixture.window, x, y, modifiers);
    }

    unsafe fn type_value(
        app: id,
        fixture: &NativeFixture,
        gui: &toybox::gpui_gui::GpuiHostedGui,
        key: &str,
        value: &str,
    ) {
        click_control(fixture, key, COMMAND);
        pump_appkit(app, gui, 0.03);
        send_key(app, fixture.window, gui, "a", 0, COMMAND);
        send_text(app, fixture.window, gui, value);
        send_key(app, fixture.window, gui, "\r", 36, 0);
        pump_appkit(app, gui, 0.03);
    }

    unsafe fn assert_delay_text_edit(
        app: id,
        fixture: &NativeFixture,
        gui: &toybox::gpui_gui::GpuiHostedGui,
        value: &str,
    ) {
        click_control(fixture, "delay-value", 0);
        pump_appkit(app, gui, 0.03);
        send_key(app, fixture.window, gui, "a", 0, COMMAND);
        send_text(app, fixture.window, gui, value);
        send_key(app, fixture.window, gui, "\r", 36, 0);
    }

    unsafe fn exercise_native_input(fixture: &NativeFixture) {
        let app = NSApp();
        let (mut gui, params, _status) = new_screenshot_gui_with_params();
        gui.set_parent_raw(fixture.parent_handle());
        assert!(gui.open(), "Pump GPUI input fixture should open");
        gui.request_resize(OUTPUT_WIDTH, OUTPUT_HEIGHT);
        let _: () = msg_send![fixture.window, makeFirstResponder: std::ptr::null_mut::<Object>()];
        pump_appkit(app, &gui, 0.1);
        capture_frame(&gui, "opened instrument editor");

        for key in [
            "timing-mode",
            "timing-value",
            "delay-value",
            "slider-Smooth",
            "slider-Swing",
            "band-3",
            "band-4",
            "knob-Mix",
            "knob-OutputGain",
            "value-Smooth",
            "value-Swing",
        ] {
            let [x, y, w, h] = pump::gui_gpui::screenshot_bounds(key);
            assert!(
                x >= 0.
                    && y >= 0.
                    && w > 0.
                    && h > 0.
                    && x + w <= OUTPUT_WIDTH as f32
                    && y + h <= OUTPUT_HEIGHT as f32,
                "{key} must remain inside the minimum editor: {x},{y},{w},{h}"
            );
        }

        // Timing, its menu, and the retained delay entry remain native controls.
        click_control(fixture, "timing-value", 0);
        pump_appkit(app, &gui, 0.04);
        click_control(fixture, "timing-sync-6", 0);
        pump_appkit(app, &gui, 0.04);
        assert_eq!(params.sync_division(), 6);
        assert_delay_text_edit(app, fixture, &gui, "4");
        assert_eq!(params.delay_beats(), 4);
        for (character, code) in [(0, 1), (127, 0), (8, 0)] {
            click_control(fixture, "delay-value", 0);
            send_key(app, fixture.window, &gui, "a", 0, COMMAND);
            send_text(app, fixture.window, &gui, "12");
            assert!(
                gui.on_key_down(character, code, 0),
                "focused Backspace must be consumed"
            );
            gui.on_key_up(character, code, 0);
            send_key(app, fixture.window, &gui, "\r", 36, 0);
            assert_eq!(params.delay_beats(), 1);
        }
        assert_delay_text_edit(app, fixture, &gui, "4");
        let pasteboard_restore = PasteboardRestore::capture();
        click_control(fixture, "delay-value", 0);
        send_key(app, fixture.window, &gui, "a", 0, COMMAND);
        send_key(app, fixture.window, &gui, "c", 8, COMMAND);
        assert_eq!(pasteboard_string(), Some("4".to_owned()));
        set_pasteboard_string("6");
        send_key(app, fixture.window, &gui, "v", 9, COMMAND);
        send_key(app, fixture.window, &gui, "\r", 36, 0);
        assert_eq!(params.delay_beats(), 6);
        drop(pasteboard_restore);

        // Smooth and Swing now admit horizontal slider drags, including outside
        // the window, instead of the old vertical knob gesture.
        for key in ["Smooth", "Swing"] {
            let [x, y, w, h] = pump::gui_gpui::screenshot_bounds(&format!("slider-{key}"));
            let y = f64::from(y + h * 0.5);
            let start = f64::from(x + 5. + (w - 10.) * 0.25);
            let end = f64::from(x + 5. + (w - 10.) * 0.75);
            send_mouse_move(fixture.window, start, y);
            send_mouse_down(fixture.window, start, y, 0);
            send_mouse_dragged(fixture.window, end, y, 0);
            send_mouse_up(fixture.window, end, y, 0);
            pump_appkit(app, &gui, 0.04);
            let value = if key == "Smooth" {
                params.smooth()
            } else {
                params.swing()
            };
            assert!(
                (value - 0.75).abs() < 0.02,
                "{key} slider must follow its horizontal track: {value}"
            );
            type_value(app, fixture, &gui, &format!("value-{key}"), "25");
            let value = if key == "Smooth" {
                params.smooth()
            } else {
                params.swing()
            };
            assert!(
                (value - 0.25).abs() < 0.001,
                "{key} must retain native numeric entry"
            );
        }
        let [x, y, _, h] = pump::gui_gpui::screenshot_bounds("slider-Smooth");
        let y = f64::from(y + h * 0.5);
        send_mouse_down(fixture.window, f64::from(x + 10.), y, 0);
        send_mouse_dragged(fixture.window, 650., 180., 0);
        assert_eq!(params.smooth(), 1.);
        send_mouse_dragged(fixture.window, -20., 200., 0);
        assert_eq!(params.smooth(), 0.);
        send_mouse_up(fixture.window, -20., 200., 0);
        let (x, y) = center("slider-Smooth");
        send_scroll_wheel(fixture.hosted_view(), fixture.window, x, y, 12., 0);
        pump_appkit(app, &gui, 0.04);
        assert!(
            params.smooth() > 0.,
            "slider wheel must affect its parameter"
        );
        send_key(app, fixture.window, &gui, "\u{f700}", 126, 0);

        // Both band bars activate DSP without a separate switch, and one Undo
        // restores the entire drag including its activation.
        for index in [3, 4] {
            params.set_effect(0, 0.);
            params.set_effect(index, 1.);
            pump_appkit(app, &gui, 0.04);
            let curve = params.editable_curve_snapshot();
            let [x, y, w, h] = pump::gui_gpui::screenshot_bounds(&format!("band-{index}"));
            let y = f64::from(y + h * 0.5);
            let start = f64::from(x + w - 5.);
            let end = f64::from(x + 5. + (w - 10.) * 0.25);
            send_mouse_down(fixture.window, start, y, 0);
            send_mouse_dragged(fixture.window, end, y, 0);
            send_mouse_up(fixture.window, end, y, 0);
            pump_appkit(app, &gui, 0.04);
            assert_eq!(params.effects()[0], 1.);
            assert!((params.effects()[index] - 0.25).abs() < 0.02);
            assert_eq!(params.editable_curve_snapshot(), curve);
            send_key(app, fixture.window, &gui, "z", 6, COMMAND);
            assert_eq!(params.effects()[0], 0.);
            assert_eq!(params.effects()[index], 1.);
            send_key(app, fixture.window, &gui, "z", 6, COMMAND | SHIFT);
            assert_eq!(params.effects()[0], 1.);
        }
        click_control(fixture, "dual-solo-low", 0);
        assert_eq!(params.effects()[5], 1.);
        click_control(fixture, "dual-solo-low", 0);
        assert_eq!(params.effects()[5], 0.);
        click_control(fixture, "dual-solo-high", 0);
        assert_eq!(params.effects()[6], 1.);
        click_control(fixture, "dual-solo-high", 0);
        assert_eq!(params.effects()[6], 0.);
        let slope = params.effects()[2];
        click_control(fixture, "dual-slope", 0);
        assert_ne!(params.effects()[2], slope);

        // Mix/Output are retained knobs with independent editable values.
        type_value(app, fixture, &gui, "value-Mix", "62");
        assert!((params.mix() - 0.62).abs() < 0.001);
        type_value(app, fixture, &gui, "value-OutputGain", "-3");
        assert!((params.output_gain_db() + 3.).abs() < 0.001);
        click_control(fixture, "timing-mode", 0);
        pump_appkit(app, &gui, 0.04);
        assert_eq!(params.timing_mode(), 1);
        click_control(fixture, "timing-value", 0);
        pump_appkit(app, &gui, 0.04);
        click_control(fixture, "timing-unit-ms", 0);
        pump_appkit(app, &gui, 0.04);
        type_value(app, fixture, &gui, "value-FreeRate", "500 ms");
        assert!((params.free_rate_hz() - 2.).abs() < 0.001);
        click_control(fixture, "timing-mode", 0);
        pump_appkit(app, &gui, 0.04);

        // Curve hit testing uses the same painted plot as the envelope.
        let origin = params.editable_curve_snapshot();
        let node = origin.nodes[1];
        let [left, top, width, height] = pump::gui_gpui::screenshot_bounds("curve-plot");
        let x = f64::from(left + node.x * (width - 1.));
        let y = f64::from(top + (1. - node.y) * (height - 1.));
        send_mouse_move_with_modifiers(fixture.window, x, y, 0);
        send_mouse_down(fixture.window, x, y, 0);
        send_mouse_dragged(fixture.window, x + 8., y - 12., 0);
        send_mouse_up(fixture.window, x + 8., y - 12., 0);
        assert_ne!(params.editable_curve_snapshot(), origin);
        send_key(app, fixture.window, &gui, "z", 6, COMMAND);
        assert_eq!(params.editable_curve_snapshot(), origin);
        // Exercise the native paint path while retaining authored data in undo.
        let paint_x = f64::from(left + width * 0.65);
        let paint_y = f64::from(top + height * 0.5);
        send_secondary_mouse_down(fixture.window, paint_x, paint_y, 0);
        send_secondary_mouse_dragged(fixture.window, paint_x + 12., paint_y - 10., 0);
        send_secondary_mouse_up(fixture.window, paint_x + 12., paint_y - 10., 0);
        assert_ne!(params.editable_curve_snapshot(), origin);
        send_key(app, fixture.window, &gui, "z", 6, COMMAND);
        assert_eq!(params.editable_curve_snapshot(), origin);
        let before = params.active_sound();
        click_control(fixture, "sound-b", 0);
        assert_ne!(params.active_sound(), before);
        click_control(fixture, "sound-a", 0);
        assert_eq!(params.active_sound(), before);

        click_control(fixture, "bypass", 0);
        assert!(params.bypassed());
        send_repeated_key(app, fixture.window, &gui, " ", 49, 0);
        assert!(
            !params.bypassed(),
            "a repeated Space press toggles bypass once"
        );
        send_key(app, fixture.window, &gui, "\r", 36, 0);
        assert!(params.bypassed());
        gui.close();
        gui.set_parent_raw(fixture.parent_handle());
        assert!(gui.open(), "Pump editor should reopen");
        pump_appkit(app, &gui, 0.05);
        assert_delay_text_edit(app, fixture, &gui, "6");
        assert_eq!(params.delay_beats(), 6);
        capture_frame(&gui, "current controls and reopened editor");
        gui.close();
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
