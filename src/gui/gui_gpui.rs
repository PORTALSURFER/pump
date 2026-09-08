//! Native GPUI editor for Pump on macOS and Windows.
//!
//! The view is a retained composition over the renderer-neutral state machine
//! in [`super::model`]. Parameter changes always go through the same semantic
//! reducer used by the curve and slot operations, so CLAP and VST3 share one
//! interaction contract.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use toybox::gpui::{
    self as gpui, canvas, div, fill, font, point, prelude::*, px, relative, rgba, size, App,
    Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, KeyDownEvent,
    LayoutId, ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Render, ShapedLine, Style, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::automation_queue::PumpAutomationQueue;
use crate::curve::{sample_editable_curve, CurveNode};
use crate::params::{
    format_plain_value_text, normalized_from_plain_value, sync_division_label, PumpParams,
    SoundSide, GLOBAL_CURVE_SLOT_COUNT, PARAM_DELAY_ID, PARAM_FREE_RATE_ID, PARAM_MIX_ID,
    PARAM_OUTPUT_GAIN_ID, PARAM_SMOOTH_ID, PARAM_SWING_ID, TIMING_MODE_FREE,
};

pub(crate) use super::model::HostParamFlushRequester;
use super::model::{
    clap_edit_sink, CurvePaintSample, CurvePreviewMessage, EditorMessage, HostParamEditSink,
    KnobMessage, NumericEntryTarget, Point as ModelPoint, PumpEditorState, Vector2,
};
use super::visual_system::{
    pump_meter_colors, pump_theme, PumpColor, PUMP_TYPOGRAPHY, PUMP_VISUAL_METRICS,
};

/// Preferred logical editor width.
pub const WINDOW_WIDTH: u32 = super::WINDOW_WIDTH;
/// Preferred logical editor height.
pub const WINDOW_HEIGHT: u32 = super::WINDOW_HEIGHT;
/// Minimum logical editor width.
pub const MIN_WINDOW_WIDTH: u32 = super::MIN_WINDOW_WIDTH;
/// Minimum logical editor height.
pub const MIN_WINDOW_HEIGHT: u32 = super::MIN_WINDOW_HEIGHT;
/// Maximum logical editor width.
pub const MAX_WINDOW_WIDTH: u32 = super::MAX_WINDOW_WIDTH;
/// Maximum logical editor height.
pub const MAX_WINDOW_HEIGHT: u32 = super::MAX_WINDOW_HEIGHT;

const CURVE_HEIGHT: f32 = 153.0;
const SURFACE_PADDING: f32 = PUMP_VISUAL_METRICS.padding;
const SURFACE_SPACING: f32 = PUMP_VISUAL_METRICS.divider;
const CURVE_GUTTER: f32 = 40.8;
const CURVE_METER_GAP: f32 = 2.72;
const CURVE_METER_WIDTH: f32 = PUMP_VISUAL_METRICS.meter_panel;
const SLOT_HEIGHT: f32 = 40.8;
const SLOT_GAP: f32 = 2.72;
const DECK_HEIGHT: f32 = PUMP_VISUAL_METRICS.deck_height;
const HEADER_HEIGHT: f32 = 45.9;
const HEADER_CONTROL_HEIGHT: f32 = 34.0;
const FOOTER_HEIGHT: f32 = PUMP_VISUAL_METRICS.label_line;
const CURVE_OFFSET_BAR_HEIGHT: f32 = 10.2;
const CURVE_OFFSET_INSET: f32 = PUMP_VISUAL_METRICS.space_8;
const CURVE_NODE_HIT_RADIUS: f32 = 10.0;
const CURVE_NODE_SIZE: f32 = 3.0;
const CURVE_STROKE_WIDTH: f32 = 1.6;
const MAX_NUMERIC_TEXT_BYTES: usize = 64;

/// Events emitted by the native numeric field. Keeping these separate from
/// the editor reducer means GPUI text/IME transport never mutates parameters
/// before the user submits a valid value.
struct NumericInputChanged;
struct NumericInputSubmitted;
struct NumericInputCanceled;
struct NumericInputStepped {
    delta: i32,
}
struct NumericInputBegan;

struct NumericInput {
    target: NumericEntryTarget,
    focus_handle: FocusHandle,
    content: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
}

impl EventEmitter<NumericInputChanged> for NumericInput {}
impl EventEmitter<NumericInputSubmitted> for NumericInput {}
impl EventEmitter<NumericInputCanceled> for NumericInput {}
impl EventEmitter<NumericInputStepped> for NumericInput {}
impl EventEmitter<NumericInputBegan> for NumericInput {}

impl NumericInput {
    fn new(target: NumericEntryTarget, content: String, cx: &mut Context<Self>) -> Self {
        Self {
            target,
            focus_handle: cx.focus_handle(),
            content,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
        }
    }

    fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    fn content(&self) -> &str {
        &self.content
    }

    fn set_content(&mut self, content: String, select_all: bool, cx: &mut Context<Self>) {
        self.content = content;
        let end = self.content.len();
        self.selected_range = if select_all { 0..end } else { end..end };
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout = None;
        self.last_bounds = None;
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(index, _)| (index < offset).then_some(index))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or(self.content.len())
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = offset.min(self.content.len());
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        for (index, ch) in self.content.char_indices() {
            if utf16_offset >= offset {
                return index;
            }
            utf16_offset += ch.len_utf16();
            if utf16_offset > offset {
                return index + ch.len_utf8();
            }
        }
        self.content.len()
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.content[..offset].chars().map(char::len_utf16).sum()
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn replace_selected_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let range = self.selected_range.clone();
        self.replace_text(range, text, window, cx);
    }

    fn replace_text(
        &mut self,
        range: Range<usize>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if range.start > range.end
            || range.end > self.content.len()
            || !self.content.is_char_boundary(range.start)
            || !self.content.is_char_boundary(range.end)
        {
            return;
        }
        let resulting_len = self
            .content
            .len()
            .saturating_sub(range.end - range.start)
            .saturating_add(text.len());
        if resulting_len > MAX_NUMERIC_TEXT_BYTES {
            return;
        }
        self.content.replace_range(range.clone(), text);
        let offset = range.start + text.len();
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.emit(NumericInputChanged);
        cx.notify();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        let offset = self
            .last_layout
            .as_ref()
            .zip(self.last_bounds.as_ref())
            .map(|(line, bounds)| {
                self.offset_to_utf16(line.closest_index_for_x(event.position.x - bounds.left()))
            })
            .unwrap_or(self.content.len());
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
        cx.emit(NumericInputBegan);
        cx.stop_propagation();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let modified = modifiers.platform || modifiers.control;
        if modified {
            match key {
                "a" => self.select_all(cx),
                "c" => {
                    if !self.selected_range.is_empty() {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            self.content[self.selected_range.clone()].to_owned(),
                        ));
                    }
                }
                "x" => {
                    if !self.selected_range.is_empty() {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            self.content[self.selected_range.clone()].to_owned(),
                        ));
                        self.replace_selected_text("", window, cx);
                    }
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.replace_selected_text(&text.replace(['\r', '\n'], " "), window, cx);
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        match key {
            "backspace" => {
                if self.selected_range.is_empty() {
                    self.select_to(self.previous_boundary(self.cursor_offset()), cx);
                }
                let range = self.selected_range.clone();
                self.replace_text(range, "", window, cx);
            }
            "delete" => {
                if self.selected_range.is_empty() {
                    self.select_to(self.next_boundary(self.cursor_offset()), cx);
                }
                let range = self.selected_range.clone();
                self.replace_text(range, "", window, cx);
            }
            "left" => {
                let offset = if self.selected_range.is_empty() {
                    self.previous_boundary(self.cursor_offset())
                } else {
                    self.selected_range.start
                };
                if modifiers.shift {
                    self.select_to(offset, cx);
                } else {
                    self.move_to(offset, cx);
                }
            }
            "right" => {
                let offset = if self.selected_range.is_empty() {
                    self.next_boundary(self.cursor_offset())
                } else {
                    self.selected_range.end
                };
                if modifiers.shift {
                    self.select_to(offset, cx);
                } else {
                    self.move_to(offset, cx);
                }
            }
            "home" => {
                if modifiers.shift {
                    self.select_to(0, cx);
                } else {
                    self.move_to(0, cx);
                }
            }
            "end" => {
                if modifiers.shift {
                    self.select_to(self.content.len(), cx);
                } else {
                    self.move_to(self.content.len(), cx);
                }
            }
            "up" => cx.emit(NumericInputStepped {
                delta: if modifiers.shift { 4 } else { 1 },
            }),
            "down" => cx.emit(NumericInputStepped {
                delta: if modifiers.shift { -4 } else { -1 },
            }),
            "enter" => cx.emit(NumericInputSubmitted),
            "escape" => cx.emit(NumericInputCanceled),
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl EntityInputHandler for NumericInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        self.replace_text(range, new_text, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        if range.start > range.end
            || range.end > self.content.len()
            || !self.content.is_char_boundary(range.start)
            || !self.content.is_char_boundary(range.end)
        {
            return;
        }
        let resulting_len = self
            .content
            .len()
            .saturating_sub(range.end - range.start)
            .saturating_add(new_text.len());
        if resulting_len > MAX_NUMERIC_TEXT_BYTES {
            return;
        }
        self.content.replace_range(range.clone(), new_text);
        self.marked_range =
            (!new_text.is_empty()).then_some(range.start..range.start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|selection| {
                let start = self.offset_from_utf16(selection.start);
                let end = self.offset_from_utf16(selection.end).max(start);
                range.start + start..range.start + end
            })
            .unwrap_or_else(|| {
                let end = range.start + new_text.len();
                end..end
            });
        self.selection_reversed = false;
        cx.emit(NumericInputChanged);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(bounds.left() + line.x_for_index(range.start), bounds.top()),
            point(bounds.left() + line.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let line = self.last_layout.as_ref()?;
        Some(self.offset_to_utf16(line.closest_index_for_x(point.x - bounds.left())))
    }
}

impl Focusable for NumericInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

struct NumericTextElement {
    input: Entity<NumericInput>,
}

struct NumericTextPrepaint {
    line: Option<ShapedLine>,
    cursor: Option<gpui::PaintQuad>,
    selection: Option<gpui::PaintQuad>,
}

impl gpui::IntoElement for NumericTextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for NumericTextElement {
    type RequestLayoutState = ();
    type PrepaintState = NumericTextPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let text = input.content.clone();
        let style = window.text_style();
        let run = TextRun {
            len: text.len(),
            font: font("Ioskeley Mono"),
            color: style.color,
            background_color: None,
            underline: input.marked_range.as_ref().map(|_| UnderlineStyle {
                color: Some(style.color),
                thickness: px(1.0),
                wavy: false,
            }),
            strikethrough: None,
        };
        let line =
            window
                .text_system()
                .shape_line(text.into(), px(PUMP_TYPOGRAPHY.value.0), &[run], None);
        let cursor_position = line.x_for_index(input.cursor_offset());
        let (selection, cursor) = if input.selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + cursor_position, bounds.top()),
                        size(px(1.0), bounds.size.height),
                    ),
                    rgba(0xd8d7d3ff),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(input.selected_range.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(input.selected_range.end),
                            bounds.bottom(),
                        ),
                    ),
                    rgba(0xe9584340),
                )),
                None,
            )
        };
        NumericTextPrepaint {
            line: Some(line),
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }
        let line = prepaint.line.take().expect("numeric text line");
        let _ = line.paint(
            bounds.origin,
            window.line_height(),
            gpui::TextAlign::Center,
            None,
            window,
            cx,
        );
        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }
        self.input.update(cx, |input, _| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for NumericInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
            .key_context("PumpNumericInput")
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_key_down(cx.listener(Self::on_key_down))
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .child(NumericTextElement { input: cx.entity() })
    }
}

fn solid(color: PumpColor) -> gpui::Rgba {
    rgba(color.packed())
}

fn text_line(
    window: &mut Window,
    text: impl Into<gpui::SharedString>,
    size: f32,
    color: PumpColor,
) -> gpui::ShapedLine {
    let text = text.into();
    let run = gpui::TextRun {
        len: text.len(),
        font: font("Ioskeley Mono"),
        color: solid(color).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text, px(size), &[run], None)
}

/// Construct the CLAP-hosted GPUI editor.
pub(crate) fn new_gui(
    params: Arc<PumpParams>,
    status: Arc<crate::GuiStatus>,
    automation_queue: Arc<PumpAutomationQueue>,
    requester: Option<HostParamFlushRequester>,
) -> toybox::gpui_gui::GpuiHostedGui {
    new_hosted_gui(
        "PumpGpuiClapEditorView",
        params,
        status,
        clap_edit_sink(automation_queue, requester),
    )
}

/// Construct the VST3-hosted editor with its format-specific edit sink.
#[cfg(feature = "vst3")]
pub(crate) fn new_gui_with_edit_sink(
    params: Arc<PumpParams>,
    status: Arc<crate::GuiStatus>,
    sink: Arc<dyn HostParamEditSink>,
) -> toybox::gpui_gui::GpuiHostedGui {
    new_hosted_gui("PumpGpuiVst3EditorView", params, status, sink)
}

fn new_hosted_gui(
    class_name: &'static str,
    params: Arc<PumpParams>,
    status: Arc<crate::GuiStatus>,
    sink: Arc<dyn HostParamEditSink>,
) -> toybox::gpui_gui::GpuiHostedGui {
    let state = Rc::new(RefCell::new(PumpEditorState::new(
        Arc::clone(&params),
        Arc::clone(&status),
        sink,
    )));
    let factory_state = Rc::clone(&state);
    let teardown_pending = Rc::new(Cell::new(false));
    let factory_teardown_pending = Rc::clone(&teardown_pending);
    let visibility_state = Rc::clone(&state);
    let visibility_teardown_pending = Rc::clone(&teardown_pending);
    let teardown_in_progress = Rc::new(Cell::new(false));
    let factory_teardown_in_progress = Rc::clone(&teardown_in_progress);
    let visibility_teardown_in_progress = Rc::clone(&teardown_in_progress);
    toybox::gpui_gui::GpuiHostedGui::new(
        class_name,
        move |_window, cx| {
            let state = Rc::clone(&factory_state);
            let teardown_pending = Rc::clone(&factory_teardown_pending);
            let teardown_in_progress = Rc::clone(&factory_teardown_in_progress);
            cx.new(move |cx| PumpEditor::new(state, teardown_pending, teardown_in_progress, cx))
                .into()
        },
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
    )
    .with_size_contract(
        (MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT),
        (WINDOW_WIDTH, WINDOW_HEIGHT),
        (MAX_WINDOW_WIDTH, MAX_WINDOW_HEIGHT),
    )
    .with_fixed_aspect_ratio()
    .with_visibility_callback(move |visible| {
        if visible {
            return;
        }
        if visibility_teardown_in_progress.get() {
            return;
        }
        if let Ok(mut state) = visibility_state.try_borrow_mut() {
            visibility_teardown_in_progress.set(true);
            state.finish_for_teardown();
            visibility_teardown_in_progress.set(false);
        } else {
            visibility_teardown_pending.set(true);
        }
    })
}

/// Native fixture factory used by the release screenshot runner.
#[cfg(feature = "screenshot-test")]
#[doc(hidden)]
pub fn new_screenshot_gui() -> toybox::gpui_gui::GpuiHostedGui {
    new_screenshot_gui_with_params().0
}

/// Construct the deterministic fixture and expose its shared parameter state
/// for native input assertions.
#[cfg(feature = "screenshot-test")]
#[doc(hidden)]
pub fn new_screenshot_gui_with_params() -> (
    toybox::gpui_gui::GpuiHostedGui,
    Arc<PumpParams>,
    Arc<crate::GuiStatus>,
) {
    let params = Arc::new(PumpParams::new());
    let status = Arc::new(crate::GuiStatus::default());
    let gui = new_hosted_gui(
        "PumpGpuiScreenshotView",
        Arc::clone(&params),
        Arc::clone(&status),
        clap_edit_sink(Arc::new(PumpAutomationQueue::default()), None),
    );
    (gui, params, status)
}

/// Seed the fixture with one coherent incoming cycle so screenshot coverage
/// exercises both the source and processed waveform layers. The production
/// audio thread uses the same writer; this helper only makes the native
/// capture deterministic without starting an audio host.
#[cfg(feature = "screenshot-test")]
#[doc(hidden)]
pub fn seed_screenshot_waveform(status: &crate::GuiStatus) {
    use crate::incoming_waveform::{IncomingWaveformWriter, INCOMING_WAVEFORM_BIN_COUNT};

    let buffer = status.incoming_waveform_buffer();
    status.set_waveform_live_mode(true);
    let mut writer = IncomingWaveformWriter::default();
    for index in 0..=(INCOMING_WAVEFORM_BIN_COUNT * 2) {
        let phase = index as f32 / (INCOMING_WAVEFORM_BIN_COUNT * 2) as f32;
        writer.begin_block(buffer);
        let carrier = (phase * std::f32::consts::TAU * 2.0).sin().abs();
        let envelope = 0.18 + 0.75 * (phase * std::f32::consts::TAU).sin().abs();
        writer.record_with_cycle_mapping_and_timing_mode(
            buffer,
            phase,
            phase,
            1.0,
            0,
            0.0,
            crate::params::TIMING_MODE_SYNC,
            carrier * envelope,
            carrier * envelope * 0.82,
        );
        writer.finish_block(buffer);
    }
    status.set_waveform_live_mode(false);
}

/// Return the initial size used by host GUI negotiation.
pub(crate) const fn preferred_window_size() -> (u32, u32) {
    (WINDOW_WIDTH, WINDOW_HEIGHT)
}

struct PumpEditor {
    state: Rc<RefCell<PumpEditorState>>,
    teardown_pending: Rc<Cell<bool>>,
    teardown_in_progress: Rc<Cell<bool>>,
    curve_bounds: Rc<RefCell<Option<Bounds<Pixels>>>>,
    numeric_inputs: Vec<Entity<NumericInput>>,
    numeric_subscriptions: Vec<gpui::Subscription>,
    curve_drag_node: Option<usize>,
    curve_drag_segment: Option<usize>,
    curve_dragging_offset: bool,
    curve_drag_start: Option<Point<Pixels>>,
    curve_dragging_marquee: bool,
    curve_dragging_paint: bool,
    active_knob: Option<NumericEntryTarget>,
    last_pointer: Option<Point<Pixels>>,
}

impl PumpEditor {
    fn new(
        state: Rc<RefCell<PumpEditorState>>,
        teardown_pending: Rc<Cell<bool>>,
        teardown_in_progress: Rc<Cell<bool>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let targets = [
            NumericEntryTarget::Mix,
            NumericEntryTarget::OutputGain,
            NumericEntryTarget::Smooth,
            NumericEntryTarget::Swing,
            NumericEntryTarget::FreeRate,
            NumericEntryTarget::Delay,
        ];
        let numeric_inputs: Vec<_> = {
            let state_ref = state.borrow();
            targets
                .into_iter()
                .map(|target| {
                    let (_, text) = knob_value(&state_ref, target);
                    cx.new(|cx| NumericInput::new(target, text, cx))
                })
                .collect()
        };
        let mut numeric_subscriptions = Vec::with_capacity(numeric_inputs.len() * 5);
        for input in &numeric_inputs {
            numeric_subscriptions.push(cx.subscribe(
                input,
                |view, input, _: &NumericInputChanged, cx| {
                    let (target, draft) = {
                        let input = input.read(cx);
                        (input.target, input.content().to_owned())
                    };
                    view.dispatch(
                        EditorMessage::NumericEntry(
                            super::model::NumericEntryMessage::DraftChanged {
                                target,
                                draft,
                                dirty: true,
                            },
                        ),
                        cx,
                    );
                },
            ));
            numeric_subscriptions.push(cx.subscribe(
                input,
                |view, input, _: &NumericInputSubmitted, cx| {
                    let (target, draft) = {
                        let input = input.read(cx);
                        (input.target, input.content().to_owned())
                    };
                    view.dispatch(
                        EditorMessage::NumericEntry(super::model::NumericEntryMessage::Commit {
                            target,
                            draft,
                        }),
                        cx,
                    );
                    view.sync_numeric_input(target, cx, false);
                },
            ));
            numeric_subscriptions.push(cx.subscribe(
                input,
                |view, input, _: &NumericInputCanceled, cx| {
                    let target = input.read(cx).target;
                    view.dispatch(
                        EditorMessage::NumericEntry(super::model::NumericEntryMessage::Cancel {
                            target,
                        }),
                        cx,
                    );
                    view.sync_numeric_input(target, cx, false);
                },
            ));
            numeric_subscriptions.push(cx.subscribe(
                input,
                |view, input, event: &NumericInputStepped, cx| {
                    let (target, delta) = (input.read(cx).target, event.delta);
                    view.step_numeric_input(target, delta, cx);
                },
            ));
            numeric_subscriptions.push(cx.subscribe(
                input,
                |view, input, _: &NumericInputBegan, cx| {
                    let target = input.read(cx).target;
                    view.begin_numeric_input(target, cx);
                },
            ));
        }
        Self {
            state,
            teardown_pending,
            teardown_in_progress,
            curve_bounds: Rc::new(RefCell::new(None)),
            numeric_inputs,
            numeric_subscriptions,
            curve_drag_node: None,
            curve_drag_segment: None,
            curve_dragging_offset: false,
            curve_drag_start: None,
            curve_dragging_marquee: false,
            curve_dragging_paint: false,
            active_knob: None,
            last_pointer: None,
        }
    }

    fn numeric_input_index(target: NumericEntryTarget) -> usize {
        match target {
            NumericEntryTarget::Mix => 0,
            NumericEntryTarget::OutputGain => 1,
            NumericEntryTarget::Smooth => 2,
            NumericEntryTarget::Swing => 3,
            NumericEntryTarget::FreeRate => 4,
            NumericEntryTarget::Delay => 5,
        }
    }

    fn sync_numeric_input(
        &self,
        target: NumericEntryTarget,
        cx: &mut Context<Self>,
        select_all: bool,
    ) {
        let (_, text) = knob_value(&self.state.borrow(), target);
        let input = self.numeric_inputs[Self::numeric_input_index(target)].clone();
        input.update(cx, |input, cx| input.set_content(text, select_all, cx));
    }

    fn begin_numeric_input(&mut self, target: NumericEntryTarget, cx: &mut Context<Self>) {
        self.dispatch(
            EditorMessage::NumericEntry(super::model::NumericEntryMessage::Begin { target }),
            cx,
        );
        self.sync_numeric_input(target, cx, true);
    }

    fn step_numeric_input(
        &mut self,
        target: NumericEntryTarget,
        delta: i32,
        cx: &mut Context<Self>,
    ) {
        let state = self.state.borrow();
        let (current, _) = knob_value(&state, target);
        let step = match target {
            NumericEntryTarget::OutputGain => {
                1.0 / (crate::params::MAX_OUTPUT_GAIN_DB - crate::params::MIN_OUTPUT_GAIN_DB)
            }
            NumericEntryTarget::FreeRate => 0.01,
            NumericEntryTarget::Delay => {
                let next = (state.params().delay_beats() as i32 + delta).clamp(
                    crate::params::MIN_DELAY_BEATS as i32,
                    crate::params::MAX_DELAY_BEATS as i32,
                ) as f32;
                let normalized = normalized_from_plain_value(PARAM_DELAY_ID, next as f64)
                    .unwrap_or(current as f64) as f32;
                drop(state);
                self.dispatch(
                    EditorMessage::Knob {
                        target,
                        message: KnobMessage::Discrete { value: normalized },
                    },
                    cx,
                );
                self.sync_numeric_input(target, cx, false);
                return;
            }
            _ => 0.01,
        };
        drop(state);
        self.dispatch(
            EditorMessage::Knob {
                target,
                message: KnobMessage::Discrete {
                    value: (current + delta as f32 * step).clamp(0.0, 1.0),
                },
            },
            cx,
        );
        self.sync_numeric_input(target, cx, false);
    }

    fn dispatch(&self, message: EditorMessage, cx: &mut Context<Self>) {
        self.state.borrow_mut().dispatch(message);
        self.drain_teardown_pending();
        cx.notify();
    }

    fn drain_teardown_pending(&self) {
        if !self.teardown_pending.replace(false) || self.teardown_in_progress.get() {
            return;
        }
        self.teardown_in_progress.set(true);
        if let Ok(mut state) = self.state.try_borrow_mut() {
            state.finish_for_teardown();
        } else {
            self.teardown_pending.set(true);
        }
        self.teardown_in_progress.set(false);
    }

    fn normalized_curve_point(&self, position: Point<Pixels>) -> Option<ModelPoint> {
        let bounds = self.curve_bounds.borrow().as_ref().copied()?;
        let left = f32::from(bounds.left()) + CURVE_GUTTER;
        let top = f32::from(bounds.top());
        let width =
            (f32::from(bounds.size.width) - CURVE_GUTTER - CURVE_METER_GAP - CURVE_METER_WIDTH)
                .max(1.0);
        let height =
            (f32::from(bounds.size.height) - CURVE_OFFSET_BAR_HEIGHT - CURVE_OFFSET_INSET).max(1.0);
        Some(ModelPoint::new(
            ((f32::from(position.x) - left) / (width - 1.0).max(1.0)).clamp(0.0, 1.0),
            (1.0 - (f32::from(position.y) - top) / (height - 1.0).max(1.0)).clamp(0.0, 1.0),
        ))
    }

    fn raw_curve_node(&self, position: Point<Pixels>) -> Option<CurveNode> {
        let display = self.normalized_curve_point(position)?;
        let phase = self.state.borrow().params().phase_offset();
        Some(CurveNode {
            x: crate::dsp::authored_curve_phase(display.x, phase),
            y: display.y,
        })
    }

    fn curve_paint_sample(&self, position: Point<Pixels>) -> Option<CurvePaintSample> {
        let display = self.normalized_curve_point(position)?;
        Some(CurvePaintSample {
            node: self.raw_curve_node(position)?,
            display_position: super::curve_paint::RectPoint {
                x: display.x,
                y: display.y,
            },
            outside: false,
        })
    }

    fn node_at(&self, position: Point<Pixels>) -> Option<usize> {
        let normalized = self.normalized_curve_point(position)?;
        let state = self.state.borrow();
        let curve = state.rendered_curve();
        let phase = state.params().phase_offset();
        let mut nearest = None;
        let mut distance = CURVE_NODE_HIT_RADIUS * CURVE_NODE_HIT_RADIUS;
        let bounds = self.curve_bounds.borrow().as_ref().copied()?;
        let left = f32::from(bounds.left()) + CURVE_GUTTER;
        let top = f32::from(bounds.top());
        let width =
            (f32::from(bounds.size.width) - CURVE_GUTTER - CURVE_METER_GAP - CURVE_METER_WIDTH)
                .max(1.0);
        let height =
            (f32::from(bounds.size.height) - CURVE_OFFSET_BAR_HEIGHT - CURVE_OFFSET_INSET).max(1.0);
        for (index, node) in curve.nodes.iter().copied().enumerate() {
            let x = (node.x - phase).rem_euclid(1.0);
            let nx = left + x * (width - 1.0).max(1.0);
            let ny = top + (1.0 - node.y) * (height - 1.0).max(1.0);
            let dx = f32::from(position.x) - nx;
            let dy = f32::from(position.y) - ny;
            let d = dx * dx + dy * dy;
            if d <= distance {
                distance = d;
                nearest = Some(index);
            }
        }
        let _ = normalized;
        nearest
    }

    fn segment_at(&self, position: Point<Pixels>) -> Option<(usize, f32)> {
        let bounds = self.curve_bounds.borrow().as_ref().copied()?;
        let state = self.state.borrow();
        let curve = state.rendered_curve();
        let phase = state.params().phase_offset();
        let left = f32::from(bounds.left()) + CURVE_GUTTER;
        let top = f32::from(bounds.top());
        let width =
            (f32::from(bounds.size.width) - CURVE_GUTTER - CURVE_METER_GAP - CURVE_METER_WIDTH)
                .max(1.0);
        let height =
            (f32::from(bounds.size.height) - CURVE_OFFSET_BAR_HEIGHT - CURVE_OFFSET_INSET).max(1.0);
        let point_for = |node: CurveNode| {
            point(
                px(left + (node.x - phase).rem_euclid(1.0) * (width - 1.0).max(1.0)),
                px(top + (1.0 - node.y) * (height - 1.0).max(1.0)),
            )
        };
        let nodes: Vec<_> = curve.nodes.iter().copied().map(point_for).collect();
        let mut nearest = None;
        let mut nearest_distance = f32::INFINITY;
        for index in 0..nodes.len().saturating_sub(1) {
            let start = nodes[index];
            let end = nodes[index + 1];
            let sx = f32::from(start.x);
            let sy = f32::from(start.y);
            let ex = f32::from(end.x);
            let ey = f32::from(end.y);
            let dx = ex - sx;
            let dy = ey - sy;
            let length_sq = dx * dx + dy * dy;
            let t = if length_sq <= f32::EPSILON {
                0.0
            } else {
                (((f32::from(position.x) - sx) * dx + (f32::from(position.y) - sy) * dy)
                    / length_sq)
                    .clamp(0.0, 1.0)
            };
            let px = sx + t * dx;
            let py = sy + t * dy;
            let distance = ((f32::from(position.x) - px).powi(2)
                + (f32::from(position.y) - py).powi(2))
            .sqrt();
            if distance < nearest_distance {
                nearest_distance = distance;
                nearest = Some(index);
            }
        }
        (nearest_distance <= 14.0).then_some((nearest?, nearest_distance))
    }

    fn curve_dimensions(&self) -> Option<Vector2> {
        let bounds = self.curve_bounds.borrow().as_ref().copied()?;
        Some(Vector2::new(
            (f32::from(bounds.size.width) - CURVE_GUTTER - CURVE_METER_GAP - CURVE_METER_WIDTH)
                .max(1.0),
            (f32::from(bounds.size.height) - CURVE_OFFSET_BAR_HEIGHT - CURVE_OFFSET_INSET).max(1.0),
        ))
    }

    fn curve_offset_pointer_x(&self, position: Point<Pixels>) -> Option<f32> {
        let bounds = self.curve_bounds.borrow().as_ref().copied()?;
        let left = f32::from(bounds.left()) + CURVE_GUTTER;
        let width =
            (f32::from(bounds.size.width) - CURVE_GUTTER - CURVE_METER_GAP - CURVE_METER_WIDTH)
                .max(1.0);
        Some(((f32::from(position.x) - left) / width).clamp(0.0, 1.0))
    }

    fn in_curve_offset_bar(&self, position: Point<Pixels>) -> bool {
        let Some(bounds) = self.curve_bounds.borrow().as_ref().copied() else {
            return false;
        };
        let left = f32::from(bounds.left()) + CURVE_GUTTER;
        let width =
            (f32::from(bounds.size.width) - CURVE_GUTTER - CURVE_METER_GAP - CURVE_METER_WIDTH)
                .max(1.0);
        let top = f32::from(bounds.top());
        let height =
            (f32::from(bounds.size.height) - CURVE_OFFSET_BAR_HEIGHT - CURVE_OFFSET_INSET).max(1.0);
        let offset_top = top + height + CURVE_OFFSET_INSET;
        let x = f32::from(position.x);
        let y = f32::from(position.y);
        x >= left
            && x <= left + width
            && y >= offset_top
            && y <= offset_top + CURVE_OFFSET_BAR_HEIGHT
    }

    fn curve_push_through_threshold(&self) -> f32 {
        let width = self
            .curve_dimensions()
            .map_or(WINDOW_WIDTH as f32, |size| size.x);
        10.0 / (width - 1.0).max(1.0)
    }

    fn curve_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event.button, MouseButton::Left | MouseButton::Right) {
            return;
        }
        self.last_pointer = Some(event.position);
        self.curve_drag_start = Some(event.position);
        self.curve_drag_segment = None;
        self.curve_dragging_offset = false;
        self.curve_dragging_marquee = false;
        let display_point = self
            .normalized_curve_point(event.position)
            .unwrap_or_default();
        let point = self
            .raw_curve_node(event.position)
            .unwrap_or(CurveNode { x: 0.0, y: 0.0 });
        let shift = event.modifiers.shift;
        let option = event.modifiers.alt;
        let command = event.modifiers.platform || event.modifiers.control;
        if self.in_curve_offset_bar(event.position)
            || (event.button == MouseButton::Left && command && shift)
        {
            if let Some(pointer_x) = self.curve_offset_pointer_x(event.position) {
                self.curve_dragging_offset = true;
                self.curve_dragging_paint = false;
                self.dispatch(
                    EditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
                        pointer_x,
                        quantized: option,
                    }),
                    cx,
                );
            }
        } else if let Some(index) = self.node_at(event.position) {
            self.curve_drag_node = Some(index);
            self.curve_dragging_paint = false;
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::PressNode {
                    index,
                    pointer: CurveNode {
                        x: point.x,
                        y: point.y,
                    },
                    shift_held: shift,
                    option_held: option,
                    command_held: command,
                }),
                cx,
            );
        } else if event.button == MouseButton::Left && shift && !option {
            self.curve_dragging_marquee = true;
            self.curve_dragging_paint = false;
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::PressMarquee {
                    start: CurveNode {
                        x: point.x,
                        y: point.y,
                    },
                }),
                cx,
            );
        } else if command {
            self.curve_dragging_paint = false;
            if let Some((index, distance)) = self.segment_at(event.position) {
                self.curve_drag_segment = Some(index);
                self.dispatch(
                    EditorMessage::Curve(CurvePreviewMessage::PressSegmentMove {
                        index,
                        position: ModelPoint::new(
                            f32::from(event.position.x),
                            f32::from(event.position.y),
                        ),
                    }),
                    cx,
                );
                let _ = distance;
            } else {
                self.dispatch(
                    EditorMessage::Curve(CurvePreviewMessage::InsertNode {
                        node: CurveNode {
                            x: point.x,
                            y: point.y,
                        },
                        command_held: true,
                    }),
                    cx,
                );
                self.curve_drag_node = self.state.borrow().active_node();
            }
        } else if let Some((index, distance)) = self.segment_at(event.position) {
            self.curve_drag_segment = Some(index);
            self.curve_dragging_paint = false;
            let position =
                ModelPoint::new(f32::from(event.position.x), f32::from(event.position.y));
            let message = if option {
                CurvePreviewMessage::PressSegment { index, position }
            } else if distance <= 4.0 {
                self.curve_drag_segment = None;
                CurvePreviewMessage::InsertNode {
                    node: CurveNode {
                        x: point.x,
                        y: point.y,
                    },
                    command_held: false,
                }
            } else {
                CurvePreviewMessage::PressDirectProximitySegment { index, position }
            };
            self.dispatch(EditorMessage::Curve(message), cx);
            if distance <= 4.0 {
                self.curve_drag_node = self.state.borrow().active_node();
            }
        } else {
            self.curve_drag_node = None;
            self.curve_dragging_paint = true;
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::PressPaint {
                    sample: self
                        .curve_paint_sample(event.position)
                        .unwrap_or(CurvePaintSample {
                            node: point,
                            display_position: super::curve_paint::RectPoint {
                                x: display_point.x,
                                y: display_point.y,
                            },
                            outside: false,
                        }),
                }),
                cx,
            );
        }
    }

    fn curve_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            event.pressed_button,
            Some(MouseButton::Left | MouseButton::Right)
        ) {
            return;
        }
        let Some(display_point) = self.normalized_curve_point(event.position) else {
            return;
        };
        let point = self
            .raw_curve_node(event.position)
            .unwrap_or(CurveNode { x: 0.0, y: 0.0 });
        self.last_pointer = Some(event.position);
        if self.curve_dragging_marquee {
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::DragMarquee {
                    current: CurveNode {
                        x: point.x,
                        y: point.y,
                    },
                }),
                cx,
            );
        } else if self.curve_dragging_offset {
            let Some(start) = self.curve_drag_start else {
                return;
            };
            let dimensions = self
                .curve_dimensions()
                .unwrap_or(Vector2::new(WINDOW_WIDTH as f32, CURVE_HEIGHT));
            let delta = (f32::from(event.position.x) - f32::from(start.x)) / dimensions.x.max(1.0);
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::DragCurveOffset { delta }),
                cx,
            );
        } else if let Some(index) = self.curve_drag_node {
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::DragNode {
                    index,
                    node: CurveNode {
                        x: point.x,
                        y: point.y,
                    },
                    push_through_threshold_x: self.curve_push_through_threshold(),
                }),
                cx,
            );
        } else if let Some(index) = self.curve_drag_segment {
            let dimensions = self
                .curve_dimensions()
                .unwrap_or(Vector2::new(WINDOW_WIDTH as f32, CURVE_HEIGHT));
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::DragSegment {
                    index,
                    position: ModelPoint::new(
                        f32::from(event.position.x),
                        f32::from(event.position.y),
                    ),
                    curve_size: dimensions,
                }),
                cx,
            );
        } else if self.curve_dragging_paint {
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::DragPaint {
                    sample: self
                        .curve_paint_sample(event.position)
                        .unwrap_or(CurvePaintSample {
                            node: point,
                            display_position: super::curve_paint::RectPoint {
                                x: display_point.x,
                                y: display_point.y,
                            },
                            outside: false,
                        }),
                }),
                cx,
            );
        }
    }

    fn curve_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event.button, MouseButton::Left | MouseButton::Right) {
            return;
        }
        let display_point = self
            .normalized_curve_point(event.position)
            .unwrap_or_default();
        let point = self
            .raw_curve_node(event.position)
            .unwrap_or(CurveNode { x: 0.0, y: 0.0 });
        if self.curve_dragging_marquee {
            self.curve_dragging_marquee = false;
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::ReleaseMarquee {
                    current: CurveNode {
                        x: point.x,
                        y: point.y,
                    },
                }),
                cx,
            );
        } else if self.curve_dragging_offset {
            self.curve_dragging_offset = false;
            let start = self.curve_drag_start.take().unwrap_or(event.position);
            let dimensions = self
                .curve_dimensions()
                .unwrap_or(Vector2::new(WINDOW_WIDTH as f32, CURVE_HEIGHT));
            let delta = (f32::from(event.position.x) - f32::from(start.x)) / dimensions.x.max(1.0);
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::ReleaseCurveOffset {
                    delta,
                    option_held: event.modifiers.alt,
                }),
                cx,
            );
        } else if let Some(index) = self.curve_drag_node.take() {
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
                    index,
                    node: CurveNode {
                        x: point.x,
                        y: point.y,
                    },
                    push_through_threshold_x: self.curve_push_through_threshold(),
                    shift_held: event.modifiers.shift,
                    option_held: event.modifiers.alt,
                    command_held: event.modifiers.platform || event.modifiers.control,
                }),
                cx,
            );
        } else if let Some(index) = self.curve_drag_segment.take() {
            let dimensions = self
                .curve_dimensions()
                .unwrap_or(Vector2::new(WINDOW_WIDTH as f32, CURVE_HEIGHT));
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::ReleaseSegment {
                    index,
                    position: ModelPoint::new(
                        f32::from(event.position.x),
                        f32::from(event.position.y),
                    ),
                    curve_size: dimensions,
                }),
                cx,
            );
        } else if self.curve_dragging_paint {
            self.curve_dragging_paint = false;
            let message = if event.button == MouseButton::Right {
                CurvePreviewMessage::ReleasePaint {
                    sample: Some(CurvePaintSample {
                        node: point,
                        display_position: super::curve_paint::RectPoint {
                            x: display_point.x,
                            y: display_point.y,
                        },
                        outside: false,
                    }),
                }
            } else {
                CurvePreviewMessage::ReleasePaint {
                    sample: Some(CurvePaintSample {
                        node: point,
                        display_position: super::curve_paint::RectPoint {
                            x: display_point.x,
                            y: display_point.y,
                        },
                        outside: false,
                    }),
                }
            };
            self.dispatch(EditorMessage::Curve(message), cx);
        }
        self.curve_drag_start = None;
        self.last_pointer = None;
    }

    fn knob_down(
        &mut self,
        target: NumericEntryTarget,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_knob = Some(target);
        self.dispatch(
            EditorMessage::Knob {
                target,
                message: KnobMessage::GestureStarted,
            },
            cx,
        );
    }

    fn knob_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let (Some(target), Some(previous)) = (self.active_knob, self.last_pointer) else {
            self.last_pointer = Some(event.position);
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let delta = (f32::from(previous.y) - f32::from(event.position.y)) * 0.004;
        self.last_pointer = Some(event.position);
        let current = self.state.borrow().params().clone();
        let normalized = match target {
            NumericEntryTarget::Mix => current.mix(),
            NumericEntryTarget::OutputGain => {
                normalized_from_plain_value(PARAM_OUTPUT_GAIN_ID, current.output_gain_db() as f64)
                    .unwrap_or(0.5) as f32
            }
            NumericEntryTarget::Smooth => current.smooth(),
            NumericEntryTarget::Swing => current.swing(),
            NumericEntryTarget::FreeRate => {
                normalized_from_plain_value(PARAM_FREE_RATE_ID, current.free_rate_hz() as f64)
                    .unwrap_or(0.5) as f32
            }
            NumericEntryTarget::Delay => {
                normalized_from_plain_value(PARAM_DELAY_ID, current.delay_beats() as f64)
                    .unwrap_or(0.0) as f32
            }
        };
        self.dispatch(
            EditorMessage::Knob {
                target,
                message: KnobMessage::ValueChanged {
                    value: (normalized + delta).clamp(0.0, 1.0),
                },
            },
            cx,
        );
    }

    fn knob_up(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.active_knob.take() {
            self.dispatch(
                EditorMessage::Knob {
                    target,
                    message: KnobMessage::GestureEnded,
                },
                cx,
            );
        }
        self.last_pointer = None;
    }

    fn toggle_bypass(
        &mut self,
        _: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(EditorMessage::ToggleBypass, cx);
    }

    fn toggle_timing(
        &mut self,
        _: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(EditorMessage::ToggleTimingMode, cx);
    }

    fn select_sound_a(
        &mut self,
        _: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(
            EditorMessage::SelectSound {
                side: SoundSide::A,
                copy: false,
            },
            cx,
        );
    }

    fn select_sound_b(
        &mut self,
        _: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(
            EditorMessage::SelectSound {
                side: SoundSide::B,
                copy: false,
            },
            cx,
        );
    }

    fn select_sound_switch(
        &mut self,
        event: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.state.borrow().params().active_sound();
        self.dispatch(
            EditorMessage::SelectSound {
                side: active.other(),
                copy: event.modifiers().platform || event.modifiers().control,
            },
            cx,
        );
    }

    fn undo(&mut self, _: &gpui::ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(EditorMessage::Undo, cx);
    }

    fn redo(&mut self, _: &gpui::ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(EditorMessage::Redo, cx);
    }

    fn toggle_timing_dropdown(
        &mut self,
        _: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(EditorMessage::ToggleTimingDropdown, cx);
    }

    fn toggle_hotkey_help(
        &mut self,
        _: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(EditorMessage::ToggleHotkeyHelp, cx);
    }

    fn toggle_waveform(
        &mut self,
        _: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(EditorMessage::ToggleWaveformMode, cx);
    }

    fn slot_click(
        &mut self,
        index: usize,
        event: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = if event.modifiers().platform || event.modifiers().control {
            super::model::CurveSlotMessage::Store { index }
        } else {
            super::model::CurveSlotMessage::Load { index }
        };
        self.dispatch(EditorMessage::CurveSlot(message), cx);
    }

    fn step_delay(&mut self, delta: i32, _window: &mut Window, cx: &mut Context<Self>) {
        self.step_numeric_input(NumericEntryTarget::Delay, delta, cx);
    }

    fn handle_modifiers(
        &mut self,
        event: &ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(
            EditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                option_held: event.modifiers.alt,
                command_held: event.modifiers.platform || event.modifiers.control,
                shift_held: event.modifiers.shift,
            }),
            cx,
        );
    }

    fn pump_tick(&mut self, window: &mut Window) {
        self.drain_teardown_pending();
        let state = self.state.borrow();
        if state.status().has_host_beats_timeline()
            || state.status().is_playing()
            || state.status().gain_reduction_needs_redraw()
        {
            window.request_animation_frame();
        }
    }
}

impl Drop for PumpEditor {
    fn drop(&mut self) {
        // A host can destroy a child view while a pointer gesture is active.
        // End/cancel the semantic gesture before releasing the retained state;
        // this preserves audio parameters while closing the native surface.
        if self.teardown_in_progress.get() {
            self.teardown_pending.set(true);
            return;
        }
        if let Ok(mut state) = self.state.try_borrow_mut() {
            if state.has_active_gesture() {
                self.teardown_in_progress.set(true);
                state.finish_for_teardown();
                self.teardown_in_progress.set(false);
            }
        } else {
            self.teardown_pending.set(true);
        }
    }
}

fn draw_waveform_layer(
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    samples: &[f32],
    color: PumpColor,
    stroke_width: f32,
    window: &mut Window,
) {
    if samples.is_empty() {
        return;
    }
    let denominator = samples.len().saturating_sub(1).max(1) as f32;
    let center_y = top + height * 0.5;
    let amplitude_scale = height * 0.43;
    let mut upper = gpui::PathBuilder::stroke(px(stroke_width));
    let mut lower = gpui::PathBuilder::stroke(px(stroke_width));
    for (index, amplitude) in samples.iter().copied().enumerate() {
        let x = left + index as f32 / denominator * (width - 1.0).max(1.0);
        let offset = amplitude.clamp(0.0, 1.0) * amplitude_scale;
        let upper_point = point(px(x), px(center_y - offset));
        let lower_point = point(px(x), px(center_y + offset));
        if index == 0 {
            upper.move_to(upper_point);
            lower.move_to(lower_point);
        } else {
            upper.line_to(upper_point);
            lower.line_to(lower_point);
        }
    }
    if let Ok(path) = upper.build() {
        window.paint_path(path, solid(color));
    }
    if let Ok(path) = lower.build() {
        window.paint_path(path, solid(color));
    }
}

fn draw_curve(bounds: Bounds<Pixels>, state: &PumpEditorState, window: &mut Window, cx: &mut App) {
    let theme = pump_theme();
    // The plot shares the primary dark surface with the baseline editor;
    // raised panels are reserved for controls and the slot row.
    window.paint_quad(fill(bounds, solid(theme.clear)));
    let left = f32::from(bounds.left()) + CURVE_GUTTER;
    let top = f32::from(bounds.top());
    let width = (f32::from(bounds.size.width) - CURVE_GUTTER - CURVE_METER_GAP - CURVE_METER_WIDTH)
        .max(1.0);
    let height =
        (f32::from(bounds.size.height) - CURVE_OFFSET_BAR_HEIGHT - CURVE_OFFSET_INSET).max(1.0);
    let curve_bounds = Bounds::from_corners(
        point(px(left), px(top)),
        point(px(left + width), px(top + height)),
    );
    let grid = super::curve_beat_grid(state.params().sync_division(), width);
    for (positions, color) in [
        (&grid.minor, theme.grid_soft),
        (&grid.major, theme.grid_strong),
    ] {
        for position in positions {
            let position = crate::dsp::swing_warp_phase(*position, state.params().swing());
            let x = left + (width - 1.0).max(1.0) * position;
            window.paint_quad(fill(
                Bounds::from_corners(point(px(x), px(top)), point(px(x + 1.0), px(top + height))),
                solid(color),
            ));
        }
    }
    for reference in crate::gui::curve_gain_references_for_mapping(
        state.params().depth_db(),
        state.params().floor_db(),
    ) {
        let y = top + (1.0 - reference.gain.clamp(0.0, 1.0)) * (height - 1.0).max(1.0);
        window.paint_quad(fill(
            Bounds::from_corners(point(px(left), px(y)), point(px(left + width), px(y + 1.0))),
            solid(theme.grid_soft),
        ));
    }
    let curve = state.rendered_curve();
    let phase = state.params().phase_offset();
    let dsp_snapshot = state.status().dsp_snapshot();
    let applied_phase = dsp_snapshot
        .map(|snapshot| snapshot.applied_phase_offset)
        .unwrap_or(phase);
    if let Some(waveform) = state.status().incoming_waveform_snapshot() {
        draw_waveform_layer(
            left,
            top,
            width,
            height,
            &waveform,
            theme.text_muted.with_alpha(88),
            1.0,
            window,
        );
        let processed = super::projection::processed_waveform(
            &curve,
            &waveform,
            applied_phase,
            state.params().depth_db(),
            state.params().floor_db(),
        );
        draw_waveform_layer(
            left,
            top,
            width,
            height,
            &processed,
            theme.accent_copper.with_alpha(96),
            2.0,
            window,
        );
    }
    if state.params().smooth() > f32::EPSILON {
        let mut smooth_path = gpui::PathBuilder::stroke(px(1.0));
        let samples = 96;
        for index in 0..=samples {
            let display_phase = index as f32 / samples as f32;
            let authored = crate::dsp::authored_curve_phase(display_phase, phase);
            let y =
                super::projection::sample_smoothed_curve(&curve, authored, state.params().smooth());
            let sample_point = point(
                px(left + display_phase * (width - 1.0).max(1.0)),
                px(top + (1.0 - y) * (height - 1.0).max(1.0)),
            );
            if index == 0 {
                smooth_path.move_to(sample_point);
            } else {
                smooth_path.line_to(sample_point);
            }
        }
        if let Ok(path) = smooth_path.build() {
            window.paint_path(path, solid(theme.accent_warning.with_alpha(180)));
        }
    }
    let mut area = gpui::PathBuilder::fill();
    let mut path = gpui::PathBuilder::stroke(px(CURVE_STROKE_WIDTH));
    let samples = 128;
    for index in 0..=samples {
        let x = index as f32 / samples as f32;
        let authored = (x + phase).rem_euclid(1.0);
        let y = sample_editable_curve(&curve, authored).clamp(0.0, 1.0);
        let point = point(
            px(left + x * (width - 1.0).max(1.0)),
            px(top + (1.0 - y) * (height - 1.0).max(1.0)),
        );
        if index == 0 {
            path.move_to(point);
            area.move_to(point);
        } else {
            path.line_to(point);
            area.line_to(point);
        }
    }
    area.line_to(point(px(left + width), px(top + height)));
    area.line_to(point(px(left), px(top + height)));
    area.close();
    if let Ok(area) = area.build() {
        window.paint_path(area, solid(theme.accent_mint.with_alpha(50)));
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, solid(theme.accent_mint));
    }
    for (index, node) in curve.nodes.iter().copied().enumerate() {
        let x = (node.x - phase).rem_euclid(1.0);
        let center = point(
            px(left + x * (width - 1.0).max(1.0)),
            px(top + (1.0 - node.y) * (height - 1.0).max(1.0)),
        );
        let node_bounds = Bounds::from_corners(
            point(
                center.x - px(CURVE_NODE_SIZE),
                center.y - px(CURVE_NODE_SIZE),
            ),
            point(
                center.x + px(CURVE_NODE_SIZE),
                center.y + px(CURVE_NODE_SIZE),
            ),
        );
        let mut node_path = gpui::PathBuilder::stroke(px(if state.selected_node(index) {
            1.35
        } else {
            1.0
        }));
        let endpoint = index == 0 || index + 1 == curve.nodes.len();
        if endpoint {
            for step in 0..=16 {
                let angle = std::f32::consts::TAU * step as f32 / 16.0;
                let node_point = point(
                    center.x + px(CURVE_NODE_SIZE * 0.7 * angle.cos()),
                    center.y + px(CURVE_NODE_SIZE * 0.7 * angle.sin()),
                );
                if step == 0 {
                    node_path.move_to(node_point);
                } else {
                    node_path.line_to(node_point);
                }
            }
        } else {
            node_path.move_to(point(node_bounds.left(), node_bounds.top()));
            node_path.line_to(point(node_bounds.right(), node_bounds.top()));
            node_path.line_to(point(node_bounds.right(), node_bounds.bottom()));
            node_path.line_to(point(node_bounds.left(), node_bounds.bottom()));
            node_path.close();
        }
        if let Ok(path) = node_path.build() {
            window.paint_path(
                path,
                solid(if state.selected_node(index) {
                    theme.accent_copper
                } else {
                    theme.accent_mint
                }),
            );
        }
    }
    if state.status().has_host_beats_timeline() || state.status().is_playing() {
        let playhead_phase = state.status().phase_from_dsp_snapshot(dsp_snapshot);
        let playhead_y = super::projection::sample_display_curve(&curve, playhead_phase, phase);
        let playhead_x = left + playhead_phase * (width - 1.0).max(1.0);
        let playhead_color = PumpColor::rgb(128, 132, 132);
        let mut playhead = gpui::PathBuilder::stroke(px(1.275));
        playhead.move_to(point(px(playhead_x), px(top)));
        playhead.line_to(point(px(playhead_x), px(top + height)));
        if let Ok(path) = playhead.build() {
            window.paint_path(path, solid(playhead_color));
        }
        let marker_y = top + (1.0 - playhead_y.clamp(0.0, 1.0)) * (height - 1.0).max(1.0);
        let mut marker = gpui::PathBuilder::fill();
        marker.move_to(point(px(playhead_x - 3.825), px(marker_y)));
        marker.line_to(point(px(playhead_x + 3.825), px(marker_y)));
        marker.line_to(point(px(playhead_x), px(marker_y + 4.25)));
        marker.close();
        if let Ok(path) = marker.build() {
            window.paint_path(path, solid(playhead_color));
        }
    }
    // Reference labels stay in the fixed gutter while the right-hand GR
    // meter remains fixed as the curve viewport expands at 1280x800.
    for reference in crate::gui::curve_gain_references_for_mapping(
        state.params().depth_db(),
        state.params().floor_db(),
    ) {
        let label = crate::gui::curve_gain_reference_text(reference, false);
        let line = text_line(window, label, PUMP_TYPOGRAPHY.meta.0, theme.text_muted);
        let y = top + (1.0 - reference.gain.clamp(0.0, 1.0)) * (height - 1.0).max(1.0)
            - PUMP_TYPOGRAPHY.meta.1 * 0.5;
        let _ = line.paint(
            point(px(f32::from(bounds.left()) + 5.0), px(y.max(top))),
            px(PUMP_TYPOGRAPHY.meta.1),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
    }
    let meter_left = left + width + CURVE_METER_GAP;
    let meter_panel = Bounds::from_corners(
        point(px(meter_left), px(top + 10.0)),
        point(px(meter_left + CURVE_METER_WIDTH), px(top + height - 10.0)),
    );
    let meter = Bounds::from_corners(
        point(meter_panel.left() + px(8.2), meter_panel.top()),
        point(
            meter_panel.left() + px(8.2 + PUMP_VISUAL_METRICS.meter_track),
            meter_panel.bottom(),
        ),
    );
    window.paint_quad(fill(meter, solid(pump_meter_colors().track)));
    window.paint_quad(fill(
        Bounds::from_corners(
            point(meter.left(), meter.top()),
            point(meter.right(), meter.top() + px(1.0)),
        ),
        solid(pump_meter_colors().border),
    ));
    window.paint_quad(fill(
        Bounds::from_corners(
            point(meter.left(), meter.bottom() - px(1.0)),
            point(meter.right(), meter.bottom()),
        ),
        solid(pump_meter_colors().border),
    ));
    let reduction = state.status().gain_reduction_db();
    let fraction = crate::gui_status::gain_reduction_meter_fraction(reduction);
    let segments = 24usize;
    let segment_step = (f32::from(meter.size.height) - 2.0) / segments as f32;
    for index in 0..segments {
        let y = f32::from(meter.bottom()) - 1.0 - (index + 1) as f32 * segment_step;
        let active = index < (fraction * segments as f32).round() as usize;
        window.paint_quad(fill(
            Bounds::from_corners(
                point(meter.left() + px(1.0), px(y)),
                point(meter.right() - px(1.0), px(y + segment_step - 1.0)),
            ),
            solid(if active {
                if fraction > 0.75 {
                    pump_meter_colors().hot
                } else {
                    pump_meter_colors().nominal
                }
            } else {
                pump_meter_colors().track
            }),
        ));
    }
    let gr_label = text_line(window, "GR dB", PUMP_TYPOGRAPHY.meta.0, theme.text_muted);
    let _ = gr_label.paint(
        point(meter.left(), px(top + 1.0)),
        px(PUMP_TYPOGRAPHY.meta.1),
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
    let offset_y = top + height + CURVE_OFFSET_INSET;
    let gr_value = text_line(
        window,
        &format!("{reduction:.1}"),
        PUMP_TYPOGRAPHY.meta.0,
        theme.text_muted,
    );
    let _ = gr_value.paint(
        point(meter.left(), px(offset_y + 0.5)),
        px(PUMP_TYPOGRAPHY.meta.1),
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
    let offset_label = text_line(window, "OFFSET", PUMP_TYPOGRAPHY.meta.0, theme.text_muted);
    let _ = offset_label.paint(
        point(px(f32::from(bounds.left()) + 5.0), px(offset_y)),
        px(PUMP_TYPOGRAPHY.meta.1),
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
    window.paint_quad(fill(
        Bounds::from_corners(
            point(px(left), px(offset_y)),
            point(px(left + width), px(offset_y + CURVE_OFFSET_BAR_HEIGHT)),
        ),
        solid(theme.grid_soft),
    ));
    let offset_x = left + phase.clamp(0.0, 1.0) * (width - PUMP_VISUAL_METRICS.space_16).max(0.0);
    window.paint_quad(fill(
        Bounds::from_corners(
            point(px(offset_x), px(offset_y)),
            point(
                px(offset_x + PUMP_VISUAL_METRICS.space_16),
                px(offset_y + CURVE_OFFSET_BAR_HEIGHT),
            ),
        ),
        solid(theme.accent_warning),
    ));
    let _ = curve_bounds;
    let _ = cx;
}

fn knob_value(state: &PumpEditorState, target: NumericEntryTarget) -> (f32, String) {
    let params = state.params();
    let (normalized, plain, id) = match target {
        NumericEntryTarget::Mix => (params.mix(), params.mix(), PARAM_MIX_ID),
        NumericEntryTarget::OutputGain => (
            normalized_from_plain_value(PARAM_OUTPUT_GAIN_ID, params.output_gain_db() as f64)
                .unwrap_or(0.5) as f32,
            params.output_gain_db(),
            PARAM_OUTPUT_GAIN_ID,
        ),
        NumericEntryTarget::Smooth => (params.smooth(), params.smooth(), PARAM_SMOOTH_ID),
        NumericEntryTarget::Swing => (params.swing(), params.swing(), PARAM_SWING_ID),
        NumericEntryTarget::FreeRate => (
            normalized_from_plain_value(PARAM_FREE_RATE_ID, params.free_rate_hz() as f64)
                .unwrap_or(0.5) as f32,
            params.free_rate_hz(),
            PARAM_FREE_RATE_ID,
        ),
        NumericEntryTarget::Delay => (
            normalized_from_plain_value(PARAM_DELAY_ID, params.delay_beats() as f64).unwrap_or(0.0)
                as f32,
            params.delay_beats() as f32,
            PARAM_DELAY_ID,
        ),
    };
    let text = format_plain_value_text(id, plain as f64).unwrap_or_else(|| format!("{plain:.2}"));
    (normalized, text)
}

fn button(id: &'static str, label: String, active: bool, width: f32) -> gpui::Stateful<gpui::Div> {
    let theme = pump_theme();
    div()
        .id(id)
        .w(px(width))
        .h(px(PUMP_VISUAL_METRICS.control_height))
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(solid(if active {
            theme.accent_mint
        } else {
            theme.border
        }))
        .bg(solid(if active {
            theme.accent_mint.with_alpha(48)
        } else {
            theme.clear
        }))
        .text_color(solid(theme.text_primary))
        .font(font("Ioskeley Mono"))
        .text_size(px(PUMP_TYPOGRAPHY.body.0))
        .line_height(px(PUMP_TYPOGRAPHY.body.1))
        .child(label)
}

#[derive(Clone, Copy)]
enum IconKind {
    ChevronLeft,
    ChevronRight,
    Copy,
    Help,
    Power,
}

fn icon_button(
    id: &'static str,
    kind: IconKind,
    active: bool,
    width: f32,
) -> gpui::Stateful<gpui::Div> {
    let theme = pump_theme();
    let color = solid(if active {
        theme.accent_copper
    } else {
        theme.text_muted
    });
    let icon = canvas(
        |_bounds, _, _| {},
        move |bounds, _, window, _cx| {
            let left = f32::from(bounds.left());
            let top = f32::from(bounds.top());
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let cx = left + width * 0.5;
            let cy = top + height * 0.5;
            let mut path = gpui::PathBuilder::stroke(px(1.25));
            let line = |path: &mut gpui::PathBuilder, x1: f32, y1: f32, x2: f32, y2: f32| {
                path.move_to(point(px(x1), px(y1)));
                path.line_to(point(px(x2), px(y2)));
            };
            match kind {
                IconKind::ChevronLeft => {
                    line(&mut path, cx + 3.2, cy - 4.0, cx - 1.5, cy);
                    line(&mut path, cx - 1.5, cy, cx + 3.2, cy + 4.0);
                }
                IconKind::ChevronRight => {
                    line(&mut path, cx - 3.2, cy - 4.0, cx + 1.5, cy);
                    line(&mut path, cx + 1.5, cy, cx - 3.2, cy + 4.0);
                }
                IconKind::Copy => {
                    line(&mut path, cx - 1.5, cy - 4.0, cx + 4.0, cy - 4.0);
                    line(&mut path, cx + 4.0, cy - 4.0, cx + 4.0, cy + 1.5);
                    line(&mut path, cx + 4.0, cy + 1.5, cx - 1.5, cy + 1.5);
                    line(&mut path, cx - 1.5, cy + 1.5, cx - 1.5, cy - 4.0);
                    line(&mut path, cx - 4.0, cy - 1.5, cx + 1.5, cy - 1.5);
                    line(&mut path, cx + 1.5, cy - 1.5, cx + 1.5, cy + 4.0);
                    line(&mut path, cx + 1.5, cy + 4.0, cx - 4.0, cy + 4.0);
                    line(&mut path, cx - 4.0, cy + 4.0, cx - 4.0, cy - 1.5);
                }
                IconKind::Help => {
                    let radius = 4.2;
                    for index in 0..=16 {
                        let angle = std::f32::consts::TAU * index as f32 / 16.0;
                        let x = cx + radius * angle.cos();
                        let y = cy + radius * angle.sin();
                        if index == 0 {
                            path.move_to(point(px(x), px(y)));
                        } else {
                            path.line_to(point(px(x), px(y)));
                        }
                    }
                    path.move_to(point(px(cx), px(cy - 1.8)));
                    path.line_to(point(px(cx), px(cy + 2.0)));
                    path.move_to(point(px(cx), px(cy + 3.5)));
                    path.line_to(point(px(cx), px(cy + 3.5)));
                }
                IconKind::Power => {
                    let radius = 4.2;
                    for index in 0..=12 {
                        let angle = std::f32::consts::PI * 0.22
                            + std::f32::consts::TAU * 0.78 * index as f32 / 12.0;
                        let x = cx + radius * angle.cos();
                        let y = cy + radius * angle.sin();
                        if index == 0 {
                            path.move_to(point(px(x), px(y)));
                        } else {
                            path.line_to(point(px(x), px(y)));
                        }
                    }
                    line(&mut path, cx, cy - 5.2, cx, cy + 0.2);
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size_full();
    button(id, String::new(), active, width).child(icon)
}

impl PumpEditor {
    fn knob_element(
        &self,
        target: NumericEntryTarget,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let label = match target {
            NumericEntryTarget::Mix => "MIX",
            NumericEntryTarget::OutputGain => "OUTPUT",
            NumericEntryTarget::Smooth => "SMOOTH",
            NumericEntryTarget::Swing => "SWING",
            NumericEntryTarget::FreeRate => "RATE",
            NumericEntryTarget::Delay => "DELAY",
        };
        let id = target.widget_key();
        let theme = pump_theme();
        let input = self.numeric_inputs[Self::numeric_input_index(target)].clone();
        let normalized = knob_value(&self.state.borrow(), target).0;
        let state_down = cx.listener(move |view: &mut Self, event: &MouseDownEvent, window, cx| {
            view.knob_down(target, event, window, cx)
        });
        let knob_canvas = canvas(
            move |_bounds, _, _| {},
            move |bounds, _, window, _cx| {
                let center = point(
                    bounds.left() + bounds.size.width * 0.5,
                    bounds.top() + bounds.size.height * 0.5,
                );
                let radius =
                    (f32::from(bounds.size.width).min(f32::from(bounds.size.height)) * 0.5 - 2.0)
                        .max(1.0);
                let start = std::f32::consts::PI * 0.75;
                let end = std::f32::consts::PI * 2.25;
                let mut track = gpui::PathBuilder::stroke(px(1.0));
                let mut active = gpui::PathBuilder::stroke(px(2.2));
                for index in 0..=48 {
                    let fraction = index as f32 / 48.0;
                    let angle = start + (end - start) * fraction;
                    let point = point(
                        center.x + px(radius * angle.cos()),
                        center.y + px(radius * angle.sin()),
                    );
                    if index == 0 {
                        track.move_to(point);
                    } else {
                        track.line_to(point);
                    }
                    if fraction <= normalized {
                        if index == 0 {
                            active.move_to(point);
                        } else {
                            active.line_to(point);
                        }
                    }
                }
                if let Ok(path) = track.build() {
                    window.paint_path(path, solid(theme.border_emphasis));
                }
                if let Ok(path) = active.build() {
                    window.paint_path(path, solid(theme.accent_mint));
                }
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(center.x - px(1.0), center.y - px(1.0)),
                        point(center.x + px(1.0), center.y + px(1.0)),
                    ),
                    solid(theme.border_emphasis),
                ));
            },
        )
        .w(px(PUMP_VISUAL_METRICS.knob))
        .h(px(PUMP_VISUAL_METRICS.knob));
        div()
            .id(id)
            .flex_1()
            .h(px(DECK_HEIGHT))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(PUMP_VISUAL_METRICS.space_4))
            .on_mouse_down(MouseButton::Left, state_down)
            .child(
                div()
                    .text_color(solid(theme.text_muted))
                    .font(font("Ioskeley Mono"))
                    .text_size(px(PUMP_TYPOGRAPHY.body.0))
                    .child(label),
            )
            .child(knob_canvas)
            .child(
                div()
                    .h(px(PUMP_TYPOGRAPHY.value.1 + 4.0))
                    .w(px(PUMP_VISUAL_METRICS.knob_column))
                    .text_color(solid(theme.text_primary))
                    .font(font("Ioskeley Mono"))
                    .text_size(px(PUMP_TYPOGRAPHY.value.0))
                    .child(input),
            )
    }
}

fn curve_slot_element(
    id: &'static str,
    curve: Option<crate::curve::EditableCurve>,
    loaded: bool,
    deviated: bool,
) -> gpui::Stateful<gpui::Div> {
    let theme = pump_theme();
    let curve_color = if deviated {
        theme.accent_danger
    } else if loaded {
        theme.accent_copper
    } else {
        theme.text_muted
    };
    let preview = canvas(
        |_bounds, _, _| {},
        move |bounds, _, window, _cx| {
            let left = f32::from(bounds.left()) + 5.0;
            let top = f32::from(bounds.top()) + 4.0;
            let width = (f32::from(bounds.size.width) - 10.0).max(1.0);
            let height = (f32::from(bounds.size.height) - 8.0).max(1.0);
            let mut path =
                gpui::PathBuilder::stroke(px(if loaded || deviated { 1.35 } else { 1.0 }));
            if let Some(curve) = curve.as_ref() {
                for index in 0..=24 {
                    let x = index as f32 / 24.0;
                    let y = sample_editable_curve(curve, x).clamp(0.0, 1.0);
                    let point = point(px(left + x * width), px(top + (1.0 - y) * height));
                    if index == 0 {
                        path.move_to(point);
                    } else {
                        path.line_to(point);
                    }
                }
            } else {
                path.move_to(point(px(left), px(top + height * 0.5)));
                path.line_to(point(px(left + width), px(top + height * 0.5)));
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, solid(curve_color));
            }
        },
    )
    .size_full();
    button(id, String::new(), loaded || deviated, 1.0)
        .flex_1()
        .h(px(SLOT_HEIGHT))
        .rounded(px(PUMP_VISUAL_METRICS.radius))
        .child(preview)
}

impl Render for PumpEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        self.pump_tick(window);
        let state = self.state.borrow();
        let theme = pump_theme();
        let params = state.params();
        let active_sound = params.active_sound();
        let bypassed = params.bypassed();
        let timing_free = params.timing_mode() == TIMING_MODE_FREE;
        let curve_bounds = Rc::clone(&self.curve_bounds);
        let draw_state = Rc::clone(&self.state);
        let curve = canvas(
            move |bounds, _, _| {
                *curve_bounds.borrow_mut() = Some(bounds);
            },
            move |bounds, _, window, cx| {
                draw_curve(bounds, &draw_state.borrow(), window, cx);
            },
        )
        .size_full();
        let curve_area = div()
            .id("curve-editor")
            .relative()
            .flex_1()
            .min_h(px(CURVE_HEIGHT))
            .border_1()
            .border_color(solid(theme.border))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::curve_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::curve_mouse_down))
            .on_mouse_move(cx.listener(Self::curve_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::curve_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::curve_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::curve_mouse_up))
            .on_mouse_up_out(MouseButton::Right, cx.listener(Self::curve_mouse_up))
            .child(curve);
        let loaded_slot = state.loaded_slot();
        let slots = div()
            .h(px(SLOT_HEIGHT))
            .w_full()
            .flex()
            .gap(px(SLOT_GAP))
            .children((0..GLOBAL_CURVE_SLOT_COUNT).map(|index| {
                let slot_curve = params.global_curve_slot_curve(index);
                let loaded = loaded_slot == Some(index);
                let deviated = loaded && params.current_curve_deviates_from_global_slot(index);
                let mut slot = curve_slot_element(
                    Box::leak(format!("curve-slot-{index}").into_boxed_str()),
                    slot_curve,
                    loaded,
                    deviated,
                );
                slot = slot.on_click(cx.listener(move |view, event, window, cx| {
                    view.slot_click(index, event, window, cx)
                }));
                slot
            }));
        let deck = div()
            .h(px(DECK_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .children([
                self.knob_element(NumericEntryTarget::Smooth, cx),
                div()
                    .id("deck-divider-smooth")
                    .w(px(PUMP_VISUAL_METRICS.divider))
                    .h(px(DECK_HEIGHT - 13.6))
                    .bg(solid(theme.grid_strong)),
                self.knob_element(NumericEntryTarget::Swing, cx),
                div()
                    .id("deck-divider-swing")
                    .w(px(PUMP_VISUAL_METRICS.divider))
                    .h(px(DECK_HEIGHT - 13.6))
                    .bg(solid(theme.grid_strong)),
                self.knob_element(NumericEntryTarget::Mix, cx),
                self.knob_element(NumericEntryTarget::OutputGain, cx),
            ]);
        let mut timing_button = button(
            "timing-mode",
            if timing_free {
                "FREE".into()
            } else {
                "SYNC".into()
            },
            timing_free,
            54.4,
        )
        .h(px(HEADER_CONTROL_HEIGHT));
        timing_button = timing_button.on_click(cx.listener(Self::toggle_timing));
        let timing_label = if timing_free {
            "Hz".to_string()
        } else {
            format!("Sync {}", sync_division_label(params.sync_division()))
        };
        let mut timing_value =
            button("timing-value", timing_label, false, 95.2).h(px(HEADER_CONTROL_HEIGHT));
        timing_value = timing_value.on_click(cx.listener(Self::toggle_timing_dropdown));
        let delay_progress_value = state.status().delay_progress();
        let delay_progress = canvas(
            |_bounds, _, _| {},
            move |bounds, _, window, _cx| {
                let progress = delay_progress_value.clamp(0.0, 1.0);
                window.paint_quad(fill(bounds, solid(theme.grid_soft)));
                if progress > 0.0 {
                    window.paint_quad(fill(
                        Bounds::from_corners(
                            point(bounds.left(), bounds.top()),
                            point(
                                bounds.left() + bounds.size.width * progress,
                                bounds.bottom(),
                            ),
                        ),
                        solid(theme.accent_copper),
                    ));
                }
            },
        )
        .w(px(66.3))
        .h(px(10.0));
        let delay_input =
            self.numeric_inputs[Self::numeric_input_index(NumericEntryTarget::Delay)].clone();
        let delay_value = div()
            .id("delay-value")
            .w(px(66.3))
            .h(px(HEADER_CONTROL_HEIGHT))
            .flex()
            .flex_col()
            .gap(px(PUMP_VISUAL_METRICS.space_4))
            .child(delay_progress)
            .child(
                div()
                    .h(px(HEADER_CONTROL_HEIGHT
                        - 10.0
                        - PUMP_VISUAL_METRICS.space_4))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(delay_input),
            );
        let timing_controls = div()
            .flex()
            .items_center()
            .gap(px(PUMP_VISUAL_METRICS.space_4))
            .child(timing_button)
            .child(timing_value)
            .child(delay_value);
        let mut undo_button =
            icon_button("undo", IconKind::ChevronLeft, false, 28.0).h(px(HEADER_CONTROL_HEIGHT));
        undo_button = undo_button.on_click(cx.listener(Self::undo));
        let mut redo_button =
            icon_button("redo", IconKind::ChevronRight, false, 28.0).h(px(HEADER_CONTROL_HEIGHT));
        redo_button = redo_button.on_click(cx.listener(Self::redo));
        let mut sound_a_button =
            button("sound-a", "A".into(), false, 28.0).h(px(HEADER_CONTROL_HEIGHT));
        if active_sound == SoundSide::A {
            sound_a_button = sound_a_button
                .border_color(solid(theme.accent_copper))
                .bg(solid(theme.clear))
                .text_color(solid(theme.accent_copper));
        }
        sound_a_button = sound_a_button.on_click(cx.listener(Self::select_sound_a));
        let mut sound_switch = icon_button(
            "sound-switch",
            if active_sound == SoundSide::A {
                IconKind::ChevronRight
            } else {
                IconKind::ChevronLeft
            },
            false,
            28.0,
        )
        .h(px(HEADER_CONTROL_HEIGHT));
        sound_switch = sound_switch.on_click(cx.listener(Self::select_sound_switch));
        let mut sound_b_button =
            button("sound-b", "B".into(), false, 28.0).h(px(HEADER_CONTROL_HEIGHT));
        if active_sound == SoundSide::B {
            sound_b_button = sound_b_button
                .border_color(solid(theme.accent_copper))
                .bg(solid(theme.clear))
                .text_color(solid(theme.accent_copper));
        }
        sound_b_button = sound_b_button.on_click(cx.listener(Self::select_sound_b));
        let mut help_button =
            button("hotkey-help", "?".into(), false, 28.0).h(px(HEADER_CONTROL_HEIGHT));
        help_button = help_button.on_click(cx.listener(Self::toggle_hotkey_help));
        let history = div()
            .flex()
            .items_center()
            .gap(px(PUMP_VISUAL_METRICS.space_4))
            .child(undo_button)
            .child(redo_button);
        let ab = div()
            .flex()
            .items_center()
            .gap(px(PUMP_VISUAL_METRICS.space_4))
            .child(sound_a_button)
            .child(sound_switch)
            .child(sound_b_button);
        let header_left = div()
            .flex()
            .items_center()
            .gap(px(PUMP_VISUAL_METRICS.gap))
            .child(timing_controls)
            .child(history)
            .child(ab);
        let brand = div()
            .flex()
            .flex_col()
            .items_end()
            .justify_center()
            .gap(px(0.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .text_color(solid(theme.text_muted))
                    .font(font("Ioskeley Mono"))
                    .text_size(px(PUMP_TYPOGRAPHY.body.0))
                    .child("PORTALSURFER / ")
                    .child(
                        div()
                            .text_color(solid(theme.accent_copper))
                            .font(font("Ioskeley Mono"))
                            .text_size(px(PUMP_TYPOGRAPHY.body.0))
                            .child("PUMP"),
                    ),
            )
            .child(
                div()
                    .text_color(solid(theme.text_muted))
                    .font(font("Ioskeley Mono"))
                    .text_size(px(PUMP_TYPOGRAPHY.meta.0))
                    .child(crate::gui::build_version_label()),
            );
        let header = div()
            .h(px(HEADER_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .child(header_left)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(PUMP_VISUAL_METRICS.gap))
                    .child(brand)
                    .child(help_button),
            );
        let mut waveform_button = button(
            "waveform-mode",
            if state.status().waveform_live_mode() {
                "LIVE".into()
            } else {
                "SYNC".into()
            },
            state.status().waveform_live_mode(),
            61.2,
        )
        .h(px(FOOTER_HEIGHT));
        waveform_button = waveform_button.on_click(cx.listener(Self::toggle_waveform));
        let mut bypass_button = button("bypass", String::new(), bypassed, 125.8)
            .h(px(FOOTER_HEIGHT))
            .flex()
            .items_center()
            .gap(px(PUMP_VISUAL_METRICS.space_8));
        bypass_button = bypass_button.child(
            canvas(
                |_bounds, _, _| {},
                move |bounds, _, window, _cx| {
                    let center = point(
                        bounds.left() + bounds.size.width * 0.5,
                        bounds.top() + bounds.size.height * 0.5,
                    );
                    let radius = 4.2;
                    let mut path = gpui::PathBuilder::stroke(px(1.25));
                    for index in 0..=12 {
                        let angle = std::f32::consts::PI * 0.22
                            + std::f32::consts::TAU * 0.78 * index as f32 / 12.0;
                        let point = point(
                            center.x + px(radius * angle.cos()),
                            center.y + px(radius * angle.sin()),
                        );
                        if index == 0 {
                            path.move_to(point);
                        } else {
                            path.line_to(point);
                        }
                    }
                    path.move_to(point(center.x, center.y - px(5.2)));
                    path.line_to(point(center.x, center.y + px(0.2)));
                    if let Ok(path) = path.build() {
                        window.paint_path(
                            path,
                            solid(if bypassed {
                                pump_theme().accent_copper
                            } else {
                                pump_theme().text_muted
                            }),
                        );
                    }
                },
            )
            .w(px(PUMP_VISUAL_METRICS.icon_hit))
            .h(px(PUMP_VISUAL_METRICS.icon_hit)),
        );
        bypass_button =
            bypass_button.child(div().child(if bypassed { "BYPASSED" } else { "ACTIVE" }));
        bypass_button = bypass_button.on_click(cx.listener(Self::toggle_bypass));
        let footer = div()
            .h(px(FOOTER_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .child(waveform_button)
            .child(bypass_button);
        div()
            .id("pump-editor")
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(SURFACE_SPACING))
            .p(px(SURFACE_PADDING))
            .bg(solid(theme.clear))
            .border_1()
            .border_color(solid(theme.border))
            .rounded(px(PUMP_VISUAL_METRICS.radius))
            .text_color(solid(theme.text_primary))
            .font(font("Ioskeley Mono"))
            .text_size(px(PUMP_TYPOGRAPHY.body.0))
            .line_height(px(PUMP_TYPOGRAPHY.body.1))
            .on_mouse_move(cx.listener(Self::knob_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::knob_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::knob_up))
            .on_modifiers_changed(cx.listener(Self::handle_modifiers))
            .child(header)
            .child(div().h(px(PUMP_VISUAL_METRICS.space_4)))
            .child(curve_area)
            .child(slots)
            .child(deck)
            .child(footer)
    }
}
