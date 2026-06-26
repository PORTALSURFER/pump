//! Minimal AppKit-backed VST3 editor for macOS hosts.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::OnceLock;

use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel, NO, YES};
use objc::{class, msg_send, sel, sel_impl};

use crate::gui::preferred_window_size;
use crate::params::PumpParams;

const NS_UTF8_STRING_ENCODING: usize = 4;
const TAG_MIX: i64 = crate::params::PARAM_MIX_NUM as i64;
const TAG_PHASE: i64 = crate::params::PARAM_PHASE_OFFSET_NUM as i64;
const TAG_OUTPUT: i64 = crate::params::PARAM_OUTPUT_GAIN_NUM as i64;
const TAG_SYNC: i64 = crate::params::PARAM_SYNC_DIVISION_NUM as i64;

#[repr(C)]
#[derive(Clone, Copy)]
struct NSPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSRect {
    origin: NSPoint,
    size: NSSize,
}

/// AppKit child editor used when Pump is loaded as a VST3 on macOS.
#[derive(Default)]
pub(super) struct CocoaPumpEditor {
    parent: Option<NonNull<c_void>>,
    root_view: Option<NonNull<Object>>,
    target: Option<NonNull<Object>>,
    params: Option<Arc<PumpParams>>,
    size: Option<(u32, u32)>,
}

impl CocoaPumpEditor {
    /// Store the AppKit parent view passed through VST3.
    pub(super) fn set_parent_raw(&mut self, parent: toybox::raw_window_handle::RawWindowHandle) {
        if let toybox::raw_window_handle::RawWindowHandle::AppKit(handle) = parent {
            self.parent = NonNull::new(handle.ns_view);
        }
    }

    /// Attach the editor view to the host parent.
    pub(super) fn open(&mut self, params: Arc<PumpParams>) -> bool {
        if self.root_view.is_some() {
            return true;
        }
        let Some(parent) = self.parent else {
            return false;
        };
        let (width, height) = preferred_window_size();
        let Some((root_view, target)) =
            (unsafe { create_editor_view(parent, &params, width, height) })
        else {
            return false;
        };
        self.root_view = Some(root_view);
        self.target = Some(target);
        self.params = Some(params);
        self.size = Some((width, height));
        true
    }

    /// Detach the editor view from the host parent.
    pub(super) fn close(&mut self) {
        unsafe {
            if let Some(root_view) = self.root_view.take() {
                let view = root_view.as_ptr();
                let _: () = msg_send![view, removeFromSuperview];
                let _: () = msg_send![view, release];
            }
            if let Some(target) = self.target.take() {
                let _: () = msg_send![target.as_ptr(), release];
            }
        }
        self.params = None;
    }

    /// Return the latest known logical editor size.
    pub(super) fn last_size(&self) -> Option<(u32, u32)> {
        self.size.or_else(|| Some(preferred_window_size()))
    }
}

impl Drop for CocoaPumpEditor {
    fn drop(&mut self) {
        self.close();
    }
}

unsafe fn create_editor_view(
    parent: NonNull<c_void>,
    params: &Arc<PumpParams>,
    width: u32,
    height: u32,
) -> Option<(NonNull<Object>, NonNull<Object>)> {
    let root_view = new_view(width as f64, height as f64)?;
    let target = new_target(Arc::as_ptr(params))?;
    install_editor_contents(root_view.as_ptr(), target.as_ptr(), params);

    let parent = parent.as_ptr().cast::<Object>();
    let _: () = msg_send![parent, addSubview: root_view.as_ptr()];
    Some((root_view, target))
}

unsafe fn new_view(width: f64, height: f64) -> Option<NonNull<Object>> {
    let view: *mut Object = msg_send![class!(NSView), alloc];
    let view: *mut Object = msg_send![view, initWithFrame: ns_rect(0.0, 0.0, width, height)];
    let view = NonNull::new(view)?;

    let _: () = msg_send![view.as_ptr(), setWantsLayer: YES];
    let layer: *mut Object = msg_send![class!(CALayer), layer];
    let color: *mut Object = msg_send![
        class!(NSColor),
        colorWithCalibratedRed: 0.055_f64
        green: 0.058_f64
        blue: 0.064_f64
        alpha: 1.0_f64
    ];
    let cg_color: *mut Object = msg_send![color, CGColor];
    let _: () = msg_send![layer, setBackgroundColor: cg_color];
    let _: () = msg_send![view.as_ptr(), setLayer: layer];

    Some(view)
}

unsafe fn new_target(params: *const PumpParams) -> Option<NonNull<Object>> {
    let target: *mut Object = msg_send![target_class(), new];
    let target = NonNull::new(target)?;
    (*target.as_ptr()).set_ivar("params", params as usize);
    Some(target)
}

unsafe fn install_editor_contents(root: *mut Object, target: *mut Object, params: &PumpParams) {
    add_label(root, "PUMP", 16.0, 240.0, 376.0, 26.0, 20.0, true);
    add_label(
        root,
        "Beat-synced gain shaper",
        16.0,
        218.0,
        376.0,
        20.0,
        12.0,
        false,
    );
    add_slider(
        root,
        target,
        "Mix",
        TAG_MIX,
        params.mix() as f64,
        0.0,
        1.0,
        178.0,
    );
    add_slider(
        root,
        target,
        "Phase",
        TAG_PHASE,
        params.phase_offset() as f64,
        0.0,
        1.0,
        132.0,
    );
    add_slider(
        root,
        target,
        "Output",
        TAG_OUTPUT,
        params.output_gain_db() as f64,
        crate::params::MIN_OUTPUT_GAIN_DB as f64,
        crate::params::MAX_OUTPUT_GAIN_DB as f64,
        86.0,
    );
    let sync_index = params.sync_division() as f64;
    add_slider(root, target, "Sync", TAG_SYNC, sync_index, 0.0, 7.0, 40.0);
}

#[allow(clippy::too_many_arguments)]
unsafe fn add_slider(
    root: *mut Object,
    target: *mut Object,
    label: &str,
    tag: i64,
    value: f64,
    min: f64,
    max: f64,
    y: f64,
) {
    add_label(root, label, 16.0, y, 64.0, 24.0, 12.0, false);
    let slider: *mut Object = msg_send![
        class!(NSSlider),
        sliderWithValue: value
        minValue: min
        maxValue: max
        target: target
        action: sel!(sliderChanged:)
    ];
    let _: () = msg_send![slider, setTag: tag];
    let _: () = msg_send![slider, setContinuous: YES];
    if tag == TAG_SYNC {
        let _: () = msg_send![slider, setNumberOfTickMarks: 8_i64];
        let _: () = msg_send![slider, setAllowsTickMarkValuesOnly: YES];
    }
    set_frame(slider, 92.0, y, 300.0, 24.0);
    let _: () = msg_send![root, addSubview: slider];
}

#[allow(clippy::too_many_arguments)]
unsafe fn add_label(
    root: *mut Object,
    text: &str,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    font_size: f64,
    bold: bool,
) {
    let label_text = ns_string(text);
    let label: *mut Object = msg_send![class!(NSTextField), labelWithString: label_text];
    let _: () = msg_send![label_text, release];
    let color: *mut Object = msg_send![
        class!(NSColor),
        colorWithCalibratedRed: 0.88_f64
        green: 0.90_f64
        blue: 0.92_f64
        alpha: 1.0_f64
    ];
    let _: () = msg_send![label, setTextColor: color];
    let font: *mut Object = if bold {
        msg_send![class!(NSFont), boldSystemFontOfSize: font_size]
    } else {
        msg_send![class!(NSFont), systemFontOfSize: font_size]
    };
    let _: () = msg_send![label, setFont: font];
    let _: () = msg_send![label, setDrawsBackground: NO];
    set_frame(label, x, y, width, height);
    let _: () = msg_send![root, addSubview: label];
}

unsafe fn set_frame(view: *mut Object, x: f64, y: f64, width: f64, height: f64) {
    let _: () = msg_send![view, setFrame: ns_rect(x, y, width.max(1.0), height.max(1.0))];
}

fn ns_rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect {
        origin: NSPoint { x, y },
        size: NSSize { width, height },
    }
}

unsafe fn ns_string(text: &str) -> *mut Object {
    let string: *mut Object = msg_send![class!(NSString), alloc];
    msg_send![
        string,
        initWithBytes: text.as_ptr()
        length: text.len()
        encoding: NS_UTF8_STRING_ENCODING
    ]
}

fn target_class() -> *const Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| {
        let superclass = class!(NSObject);
        let mut decl =
            ClassDecl::new("PumpCocoaEditorTarget", superclass).expect("unique class name");
        decl.add_ivar::<usize>("params");
        unsafe {
            decl.add_method(
                sel!(sliderChanged:),
                slider_changed as extern "C" fn(&Object, Sel, *mut Object),
            );
        }
        decl.register() as *const Class as usize
    }) as *const Class
}

extern "C" fn slider_changed(this: &Object, _cmd: Sel, sender: *mut Object) {
    unsafe {
        let params = *this.get_ivar::<usize>("params") as *const PumpParams;
        if params.is_null() || sender.is_null() {
            return;
        }
        let tag: i64 = msg_send![sender, tag];
        let value: f64 = msg_send![sender, doubleValue];
        let params = &*params;
        match tag {
            TAG_MIX => params.set_mix(value as f32),
            TAG_PHASE => params.set_phase_offset(value as f32),
            TAG_OUTPUT => params.set_output_gain_db(value as f32),
            TAG_SYNC => params.set_sync_division(value as f32),
            _ => {}
        }
    }
}
