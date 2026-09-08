//! Render Pump through the live native GPUI host and write PNG captures.

#![cfg(target_os = "macos")]

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
) {
    unsafe { pump_appkit(app, gui, 0.10) };
    let (width, height, pixels) = gui.capture_rgba().expect("live GPUI capture");
    eprintln!("{name}: captured {width}x{height}");
    write_capture(
        root,
        name,
        width,
        height,
        pixels,
        output_width,
        output_height,
    );
    let _ = fixture;
}

unsafe fn send_mouse_event(
    window: id,
    width: u32,
    height: u32,
    event_type: usize,
    x: f64,
    top_y: f64,
) {
    let window_number: isize = msg_send![window, windowNumber];
    let location = NSPoint::new(x, f64::from(height) - top_y);
    let event: id = msg_send![
        class!(NSEvent),
        mouseEventWithType: event_type
        location: location
        modifierFlags: 0_u64
        timestamp: 0.0_f64
        windowNumber: window_number
        context: std::ptr::null_mut::<Object>()
        eventNumber: 1_isize
        clickCount: 1_isize
        pressure: 1.0_f64
    ];
    let _: () = msg_send![window, sendEvent: event];
}

unsafe fn send_mouse_move(window: id, width: u32, height: u32, x: f64, top_y: f64) {
    send_mouse_event(window, width, height, 5, x, top_y);
}

unsafe fn send_mouse_down(window: id, width: u32, height: u32, x: f64, top_y: f64) {
    send_mouse_event(window, width, height, 1, x, top_y);
}

unsafe fn send_mouse_up(window: id, width: u32, height: u32, x: f64, top_y: f64) {
    send_mouse_event(window, width, height, 2, x, top_y);
}

unsafe fn send_click(window: id, width: u32, height: u32, x: f64, top_y: f64) {
    send_mouse_down(window, width, height, x, top_y);
    send_mouse_up(window, width, height, x, top_y);
}

fn main() {
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

        capture(app, &fixture, &gui, &root, "pump-default-640x400", 640, 400);
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
        send_mouse_move(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 110.0, 32.0);
        capture(
            app,
            &fixture,
            &gui,
            &root,
            "pump-header-hovered-640x400",
            640,
            400,
        );
        send_mouse_move(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 351.0, 32.0);
        capture(
            app,
            &fixture,
            &gui,
            &root,
            "pump-header-copy-hovered-640x400",
            640,
            400,
        );
        send_mouse_move(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 319.0, 32.0);
        capture(
            app,
            &fixture,
            &gui,
            &root,
            "pump-header-a-hovered-640x400",
            640,
            400,
        );
        send_mouse_move(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 383.0, 32.0);
        capture(
            app,
            &fixture,
            &gui,
            &root,
            "pump-header-b-hovered-640x400",
            640,
            400,
        );
        send_mouse_down(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 110.0, 32.0);
        capture(
            app,
            &fixture,
            &gui,
            &root,
            "pump-header-pressed-640x400",
            640,
            400,
        );
        send_mouse_up(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 110.0, 32.0);
        send_mouse_move(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 20.0, 20.0);
        capture(
            app,
            &fixture,
            &gui,
            &root,
            "pump-header-disabled-640x400",
            640,
            400,
        );

        send_click(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 319.0, 32.0);
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
        capture(app, &fixture, &gui, &root, "pump-max-1280x800", 1280, 800);
        gui.request_resize(640, 400);
        capture(app, &fixture, &gui, &root, "pump-min-640x400", 640, 400);

        // Keep the fractional-size reference explicit: Toybox owns the
        // native display scale, while this capture is a live 800x500 logical
        // raster used for responsive-layout comparison.
        gui.request_resize(800, 500);
        capture(
            app,
            &fixture,
            &gui,
            &root,
            "pump-default-640x400-dpi-1_25",
            800,
            500,
        );
        gui.request_resize(640, 400);

        // The old component fixture used a 2:1 gallery size. Preserve those
        // names with two live GPUI scenes while keeping the production host
        // contract at 8:5 for every normal capture.
        gui.close();
        gui = gui.with_size_contract((1, 1), (720, 360), (720, 360));
        gui.set_parent_raw(fixture.parent_handle());
        assert!(gui.open(), "Pump GPUI component fixture should open");
        gui.request_resize(720, 360);
        capture(
            app,
            &fixture,
            &gui,
            &root,
            "pump-components-states-720x360-1x",
            720,
            360,
        );
        send_mouse_move(fixture.window, CAPTURE_WIDTH, CAPTURE_HEIGHT, 110.0, 32.0);
        capture(
            app,
            &fixture,
            &gui,
            &root,
            "pump-components-states-720x360-2x",
            720,
            360,
        );
        gui.close();
        gui = gui.with_size_contract((640, 400), (640, 400), (1280, 800));
        gui.set_parent_raw(fixture.parent_handle());
        assert!(
            gui.open(),
            "Pump GPUI editor should reopen after gallery fixture"
        );
        gui.request_resize(640, 400);

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
        capture(
            app,
            &fixture,
            &gui,
            &root,
            "pump-non-default-active-meter-640x400",
            640,
            400,
        );

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

#[cfg(not(target_os = "macos"))]
fn main() {}
