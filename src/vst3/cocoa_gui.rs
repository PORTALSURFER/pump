//! Radiant-backed AppKit VST3 editor for macOS hosts.

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::Arc;
use std::sync::OnceLock;

use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel, BOOL, YES};
use objc::{class, msg_send, sel, sel_impl, Encode, Encoding};
use radiant::gui::types::{Point, Rect as UiRect, Rgba8};
use radiant::runtime::{Event, PaintPrimitive, PaintTextAlign, SurfacePaintPlan};
use radiant::widgets::PointerButton;

use crate::gui::{preferred_window_size, RadiantPumpEditor};
use crate::params::PumpParams;

const NS_UTF8_STRING_ENCODING: usize = 4;

#[link(name = "AppKit", kind = "framework")]
extern "C" {
    static NSFontAttributeName: *mut Object;
    static NSForegroundColorAttributeName: *mut Object;
    static NSParagraphStyleAttributeName: *mut Object;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSPoint {
    x: f64,
    y: f64,
}

unsafe impl Encode for NSPoint {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGPoint=dd}") }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSSize {
    width: f64,
    height: f64,
}

unsafe impl Encode for NSSize {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGSize=dd}") }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSRect {
    origin: NSPoint,
    size: NSSize,
}

unsafe impl Encode for NSRect {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
    }
}

/// AppKit child editor used when Pump is loaded as a VST3 on macOS.
#[derive(Default)]
pub(super) struct CocoaPumpEditor {
    parent: Option<NonNull<c_void>>,
    root_view: Option<NonNull<Object>>,
    size: Cell<Option<(u32, u32)>>,
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
        let Some(root_view) = (unsafe { create_editor_view(parent, params, width, height) }) else {
            return false;
        };
        self.root_view = Some(root_view);
        self.size.set(Some((width, height)));
        true
    }

    /// Detach the editor view from the host parent.
    pub(super) fn close(&mut self) {
        unsafe {
            if let Some(root_view) = self.root_view.take() {
                drop_runtime(root_view.as_ptr());
                let view = root_view.as_ptr();
                let _: () = msg_send![view, removeFromSuperview];
                let _: () = msg_send![view, release];
            }
        }
        self.size.set(None);
    }

    /// Return the latest known logical editor size.
    pub(super) fn last_size(&self) -> Option<(u32, u32)> {
        self.size.get().or_else(|| Some(preferred_window_size()))
    }

    /// Apply a host-driven resize to the hosted child view.
    pub(super) fn request_resize(&self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        self.size.set(Some((width, height)));
        let Some(root_view) = self.root_view else {
            return;
        };
        unsafe {
            set_frame(root_view.as_ptr(), 0.0, 0.0, width as f64, height as f64);
            if let Some(runtime) = runtime_mut(root_view.as_ptr()) {
                runtime.resize(width, height);
            }
            let _: () = msg_send![root_view.as_ptr(), setNeedsDisplay: YES];
        }
    }
}

impl Drop for CocoaPumpEditor {
    fn drop(&mut self) {
        self.close();
    }
}

unsafe fn create_editor_view(
    parent: NonNull<c_void>,
    params: Arc<PumpParams>,
    width: u32,
    height: u32,
) -> Option<NonNull<Object>> {
    let root_view = new_radiant_view(RadiantPumpEditor::new(params, width, height), width, height)?;
    let parent = parent.as_ptr().cast::<Object>();
    let _: () = msg_send![parent, addSubview: root_view.as_ptr()];
    Some(root_view)
}

unsafe fn new_radiant_view(
    runtime: RadiantPumpEditor,
    width: u32,
    height: u32,
) -> Option<NonNull<Object>> {
    let view: *mut Object = msg_send![editor_view_class(), alloc];
    let view: *mut Object =
        msg_send![view, initWithFrame: ns_rect(0.0, 0.0, width as f64, height as f64)];
    let view = NonNull::new(view)?;
    (*view.as_ptr()).set_ivar("runtime", Box::into_raw(Box::new(runtime)) as usize);
    Some(view)
}

unsafe fn render_paint_plan(plan: &SurfacePaintPlan, bounds: NSRect) {
    fill_ns_rect(
        ns_rect(0.0, 0.0, bounds.size.width, bounds.size.height),
        plan.clear_color,
    );
    for primitive in &plan.primitives {
        match primitive {
            PaintPrimitive::ClipStart(clip) => {
                let _: () = msg_send![class!(NSGraphicsContext), saveGraphicsState];
                let path: *mut Object =
                    msg_send![class!(NSBezierPath), bezierPathWithRect: ns_rect_from_ui(clip.rect)];
                let _: () = msg_send![path, addClip];
            }
            PaintPrimitive::ClipEnd(_) => {
                let _: () = msg_send![class!(NSGraphicsContext), restoreGraphicsState];
            }
            PaintPrimitive::FillRect(fill) => fill_ns_rect(ns_rect_from_ui(fill.rect), fill.color),
            PaintPrimitive::FillRectBatch(batch) => {
                for rect in batch.rects.iter().copied() {
                    fill_ns_rect(ns_rect_from_ui(rect), batch.color);
                }
            }
            PaintPrimitive::StrokeRect(stroke) => {
                stroke_ns_rect(ns_rect_from_ui(stroke.rect), stroke.color, stroke.width);
            }
            PaintPrimitive::StrokeRectBatch(batch) => {
                for rect in batch.rects.iter().copied() {
                    stroke_ns_rect(ns_rect_from_ui(rect), batch.color, batch.width);
                }
            }
            PaintPrimitive::StrokePolyline(polyline) => {
                stroke_ns_polyline(&polyline.points, polyline.color, polyline.width);
            }
            PaintPrimitive::Text(text_run) => draw_text_run(text_run),
            PaintPrimitive::OverlayPanel(panel) => {
                fill_ns_rect(ns_rect_from_ui(panel.rect), Rgba8::new(32, 32, 32, 238));
                stroke_ns_rect(
                    ns_rect_from_ui(panel.rect),
                    Rgba8::new(88, 88, 88, 255),
                    1.0,
                );
                if let Some(label) = &panel.label {
                    let rect = panel.rect;
                    draw_string(
                        label.as_str(),
                        rect,
                        12.0,
                        Rgba8::new(238, 238, 238, 255),
                        0,
                    );
                }
            }
            PaintPrimitive::TextInput(input) => {
                draw_string(
                    input.state.value.as_str(),
                    input.rect,
                    input.font_size,
                    input.color,
                    0,
                );
            }
            PaintPrimitive::FillPath(_)
            | PaintPrimitive::Svg(_)
            | PaintPrimitive::FillPolygon(_)
            | PaintPrimitive::StrokePolygon(_)
            | PaintPrimitive::Image(_)
            | PaintPrimitive::GpuSurface(_)
            | PaintPrimitive::CustomSurface(_) => {}
        }
    }
}

unsafe fn draw_text_run(text_run: &radiant::runtime::PaintTextRun) {
    let alignment = match text_run.align {
        PaintTextAlign::Left => 0,
        PaintTextAlign::Center => 1,
        PaintTextAlign::Right => 2,
    };
    draw_string(
        text_run.text.as_str(),
        text_run.rect,
        text_run.font_size,
        text_run.color,
        alignment,
    );
}

unsafe fn draw_string(text: &str, rect: UiRect, font_size: f32, color: Rgba8, alignment: i64) {
    if text.is_empty() {
        return;
    }
    let text_string = ns_string(text);
    let attrs = text_attributes(color, font_size, alignment);
    let _: () = msg_send![
        text_string,
        drawInRect: ns_rect_from_ui(rect)
        withAttributes: attrs
    ];
    let _: () = msg_send![text_string, release];
}

unsafe fn text_attributes(color: Rgba8, font_size: f32, alignment: i64) -> *mut Object {
    let color = ns_color(color);
    let font: *mut Object = msg_send![class!(NSFont), systemFontOfSize: font_size.max(8.0) as f64];
    let paragraph_style: *mut Object = msg_send![class!(NSMutableParagraphStyle), new];
    let _: () = msg_send![paragraph_style, setAlignment: alignment];
    let objects = [font, color, paragraph_style];
    let keys = [
        NSFontAttributeName,
        NSForegroundColorAttributeName,
        NSParagraphStyleAttributeName,
    ];
    let attributes: *mut Object = msg_send![
        class!(NSDictionary),
        dictionaryWithObjects: objects.as_ptr()
        forKeys: keys.as_ptr()
        count: objects.len()
    ];
    let _: () = msg_send![paragraph_style, release];
    attributes
}

unsafe fn fill_ns_rect(rect: NSRect, color: Rgba8) {
    let color = ns_color(color);
    let _: () = msg_send![color, setFill];
    let path: *mut Object = msg_send![class!(NSBezierPath), bezierPathWithRect: rect];
    let _: () = msg_send![path, fill];
}

unsafe fn stroke_ns_rect(rect: NSRect, color: Rgba8, width: f32) {
    let color = ns_color(color);
    let _: () = msg_send![color, setStroke];
    let path: *mut Object = msg_send![class!(NSBezierPath), bezierPathWithRect: rect];
    let _: () = msg_send![path, setLineWidth: width.max(0.5) as f64];
    let _: () = msg_send![path, stroke];
}

unsafe fn stroke_ns_polyline(points: &[Point], color: Rgba8, width: f32) {
    let Some(first) = points.first().copied() else {
        return;
    };
    let color = ns_color(color);
    let _: () = msg_send![color, setStroke];
    let path: *mut Object = msg_send![class!(NSBezierPath), bezierPath];
    let _: () = msg_send![path, setLineWidth: width.max(0.5) as f64];
    let _: () = msg_send![path, moveToPoint: ns_point_from_ui(first)];
    for point in points.iter().copied().skip(1) {
        let _: () = msg_send![path, lineToPoint: ns_point_from_ui(point)];
    }
    let _: () = msg_send![path, stroke];
}

unsafe fn ns_color(color: Rgba8) -> *mut Object {
    msg_send![
        class!(NSColor),
        colorWithCalibratedRed: color.r as f64 / 255.0
        green: color.g as f64 / 255.0
        blue: color.b as f64 / 255.0
        alpha: color.a as f64 / 255.0
    ]
}

unsafe fn set_frame(view: *mut Object, x: f64, y: f64, width: f64, height: f64) {
    let _: () = msg_send![view, setFrame: ns_rect(x, y, width.max(1.0), height.max(1.0))];
}

fn ns_rect_from_ui(rect: UiRect) -> NSRect {
    ns_rect(
        rect.min.x as f64,
        rect.min.y as f64,
        rect.width().max(1.0) as f64,
        rect.height().max(1.0) as f64,
    )
}

fn ns_point_from_ui(point: Point) -> NSPoint {
    NSPoint {
        x: point.x as f64,
        y: point.y as f64,
    }
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

fn editor_view_class() -> *const Class {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| {
        let superclass = class!(NSView);
        let mut decl =
            ClassDecl::new("PumpRadiantEditorView", superclass).expect("unique class name");
        decl.add_ivar::<usize>("runtime");
        unsafe {
            decl.add_method(
                sel!(drawRect:),
                draw_rect as extern "C" fn(&Object, Sel, NSRect),
            );
            decl.add_method(
                sel!(mouseDown:),
                mouse_down as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(mouseDragged:),
                mouse_dragged as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(mouseUp:),
                mouse_up as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(rightMouseDown:),
                right_mouse_down as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(rightMouseUp:),
                right_mouse_up as extern "C" fn(&Object, Sel, *mut Object),
            );
            decl.add_method(
                sel!(isFlipped),
                is_flipped as extern "C" fn(&Object, Sel) -> BOOL,
            );
            decl.add_method(
                sel!(acceptsFirstResponder),
                accepts_first_responder as extern "C" fn(&Object, Sel) -> BOOL,
            );
            decl.add_method(
                sel!(acceptsFirstMouse:),
                accepts_first_mouse as extern "C" fn(&Object, Sel, *mut Object) -> BOOL,
            );
            decl.add_method(sel!(dealloc), dealloc as extern "C" fn(&Object, Sel));
        }
        decl.register() as *const Class as usize
    }) as *const Class
}

extern "C" fn is_flipped(_this: &Object, _cmd: Sel) -> BOOL {
    YES
}

extern "C" fn accepts_first_responder(_this: &Object, _cmd: Sel) -> BOOL {
    YES
}

extern "C" fn accepts_first_mouse(_this: &Object, _cmd: Sel, _event: *mut Object) -> BOOL {
    YES
}

extern "C" fn draw_rect(this: &Object, _cmd: Sel, _dirty: NSRect) {
    unsafe {
        let bounds: NSRect = msg_send![this, bounds];
        if let Some(runtime) = runtime_mut(this) {
            render_paint_plan(runtime.paint_plan(), bounds);
        } else {
            fill_ns_rect(bounds, Rgba8::new(24, 24, 24, 255));
        }
    }
}

extern "C" fn mouse_down(this: &Object, _cmd: Sel, event: *mut Object) {
    dispatch_mouse_event(this, event, PointerButton::Primary, MouseEventKind::Press);
}

extern "C" fn mouse_dragged(this: &Object, _cmd: Sel, event: *mut Object) {
    dispatch_mouse_event(this, event, PointerButton::Primary, MouseEventKind::Move);
}

extern "C" fn mouse_up(this: &Object, _cmd: Sel, event: *mut Object) {
    dispatch_mouse_event(this, event, PointerButton::Primary, MouseEventKind::Release);
}

extern "C" fn right_mouse_down(this: &Object, _cmd: Sel, event: *mut Object) {
    dispatch_mouse_event(this, event, PointerButton::Secondary, MouseEventKind::Press);
}

extern "C" fn right_mouse_up(this: &Object, _cmd: Sel, event: *mut Object) {
    dispatch_mouse_event(
        this,
        event,
        PointerButton::Secondary,
        MouseEventKind::Release,
    );
}

extern "C" fn dealloc(this: &Object, _cmd: Sel) {
    unsafe {
        drop_runtime(this);
        let superclass = class!(NSView);
        let _: () = msg_send![super(this, superclass), dealloc];
    }
}

#[derive(Clone, Copy)]
enum MouseEventKind {
    Press,
    Move,
    Release,
}

fn dispatch_mouse_event(
    this: &Object,
    event: *mut Object,
    button: PointerButton,
    kind: MouseEventKind,
) {
    unsafe {
        if event.is_null() {
            return;
        }
        let Some(runtime) = runtime_mut(this) else {
            return;
        };
        let position = event_position(this, event);
        let event = match kind {
            MouseEventKind::Press => Event::pointer_press(position, button, Default::default()),
            MouseEventKind::Move => Event::pointer_move(position),
            MouseEventKind::Release => Event::pointer_release(position, button, Default::default()),
        };
        runtime.dispatch_event(event);
        let _: () = msg_send![this, setNeedsDisplay: YES];
    }
}

unsafe fn event_position(this: &Object, event: *mut Object) -> Point {
    let window_point: NSPoint = msg_send![event, locationInWindow];
    let local_point: NSPoint =
        msg_send![this, convertPoint: window_point fromView: ptr::null_mut::<Object>()];
    Point::new(local_point.x as f32, local_point.y as f32)
}

unsafe fn runtime_mut(view: *const Object) -> Option<&'static mut RadiantPumpEditor> {
    let runtime = *(view.as_ref()?.get_ivar::<usize>("runtime")) as *mut RadiantPumpEditor;
    runtime.as_mut()
}

unsafe fn drop_runtime(view: *const Object) {
    let Some(view) = view.cast_mut().as_mut() else {
        return;
    };
    let runtime = *view.get_ivar::<usize>("runtime") as *mut RadiantPumpEditor;
    if runtime.is_null() {
        return;
    }
    (*view).set_ivar("runtime", 0_usize);
    drop(Box::from_raw(runtime));
}
