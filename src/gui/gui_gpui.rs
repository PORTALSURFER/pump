//! Native GPUI editor for Pump on macOS and Windows.
//!
//! The view is a retained composition over the renderer-neutral state machine
//! in [`super::model`]. Parameter changes always go through the same semantic
//! reducer used by the curve and slot operations, so CLAP and VST3 share one
//! interaction contract.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use toybox::gpui::{
    self as gpui, canvas, div, fill, font, point, prelude::*, px, relative, rgba, size, App,
    Bounds, ClipboardItem, Context, CursorStyle, DispatchPhase, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, KeyDownEvent, KeyUpEvent, LayoutId, ModifiersChangedEvent, MouseButton,
    MouseDownEvent, MouseExitEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render,
    ScrollWheelEvent, ShapedLine, Style, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::automation_queue::PumpAutomationQueue;
use crate::curve::{sample_editable_curve, CurveNode};
use crate::params::{
    format_plain_value_text, normalized_from_plain_value, sync_division_label, PumpParams,
    SoundSide, GLOBAL_CURVE_SLOT_COUNT, PARAM_DELAY_ID, PARAM_FREE_RATE_ID, PARAM_MIX_ID,
    PARAM_OUTPUT_GAIN_ID, PARAM_SMOOTH_ID, PARAM_SWING_ID, SYNC_DIVISIONS, TIMING_MODE_FREE,
};

pub(crate) use super::model::HostParamFlushRequester;
use super::model::{
    clap_edit_sink, CurvePaintSample, CurvePreviewMessage, EditorMessage, FreeRateUnit,
    HostParamEditSink, KnobMessage, NumericEntryMessage, NumericEntryTarget, Point as ModelPoint,
    PumpEditorState, Vector2,
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
const CURVE_SEGMENT_MOVE_COLOR: PumpColor = PumpColor::rgb(96, 176, 255);
const CURVE_OFFSET_MOVE_COLOR: PumpColor = PumpColor::rgb(255, 168, 88);
const CURVE_OFFSET_HOVER_COLOR: PumpColor = CURVE_OFFSET_MOVE_COLOR.with_alpha(224);
const CURVE_PAINT_PREVIEW_WIDTH: f32 = 2.25;
const OPTION_GESTURE_DRAG_START_DISTANCE: f32 = 7.0;
const OPTION_GESTURE_DRAG_START_DISTANCE_SQUARED: f32 =
    OPTION_GESTURE_DRAG_START_DISTANCE * OPTION_GESTURE_DRAG_START_DISTANCE;
const MAX_NUMERIC_TEXT_BYTES: usize = 64;
const CURVE_SLOT_IDS: [&str; GLOBAL_CURVE_SLOT_COUNT] = [
    "curve-slot-0",
    "curve-slot-1",
    "curve-slot-2",
    "curve-slot-3",
    "curve-slot-4",
    "curve-slot-5",
    "curve-slot-6",
    "curve-slot-7",
];
const TIMING_FREE_RATE_IDS: [&str; 4] = [
    "timing-unit-ms",
    "timing-unit-s",
    "timing-unit-Hz",
    "timing-unit-kHz",
];
const TIMING_SYNC_IDS: [&str; 10] = [
    "timing-sync-0",
    "timing-sync-1",
    "timing-sync-2",
    "timing-sync-3",
    "timing-sync-4",
    "timing-sync-5",
    "timing-sync-6",
    "timing-sync-7",
    "timing-sync-8",
    "timing-sync-9",
];

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
    blur_subscription: Option<gpui::Subscription>,
    editing: bool,
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
            blur_subscription: None,
            editing: false,
            content,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
        }
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

    fn set_editing(&mut self, editing: bool) {
        self.editing = editing;
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

    fn valid_replacement(&self, range: &Range<usize>, text: &str) -> bool {
        if self.target != NumericEntryTarget::Delay {
            return true;
        }
        if !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        let mut candidate = String::with_capacity(
            self.content
                .len()
                .saturating_sub(range.end.saturating_sub(range.start))
                .saturating_add(text.len()),
        );
        candidate.push_str(&self.content[..range.start]);
        candidate.push_str(text);
        candidate.push_str(&self.content[range.end..]);
        candidate.is_empty()
            || candidate
                .parse::<usize>()
                .ok()
                .is_some_and(|value| value <= crate::params::MAX_DELAY_BEATS)
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
            || !self.valid_replacement(&range, text)
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
        // Delay has always been directly editable. Other knobs preserve the
        // editor's Cmd/Ctrl-click contract: a plain click starts the knob
        // gesture on the parent deck, while an explicit modifier enters the
        // native text field. Once a field is active, normal caret clicks and
        // Shift selection remain available.
        if self.target != NumericEntryTarget::Delay
            && !self.editing
            && !event.modifiers.platform
            && !event.modifiers.control
        {
            return;
        }
        self.editing = true;
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
            "back" | "backspace" => {
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
            || !self.valid_replacement(&range, new_text)
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
        let (selection, cursor) = if !input.editing {
            (None, None)
        } else if input.selected_range.is_empty() {
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
        if self.input.read(cx).editing {
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(bounds, self.input.clone()),
                cx,
            );
        }
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        if self.blur_subscription.is_none() {
            let focus_handle = self.focus_handle.clone();
            self.blur_subscription = Some(cx.on_blur(&focus_handle, window, |input, _, cx| {
                if input.editing {
                    input.editing = false;
                    cx.emit(NumericInputCanceled);
                    cx.notify();
                }
            }));
        }
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
    let pointer_cancel_pending = Rc::new(Cell::new(false));
    let factory_pointer_cancel_pending = Rc::clone(&pointer_cancel_pending);
    let pointer_cancel_state = Rc::clone(&state);
    toybox::gpui_gui::GpuiHostedGui::new(
        class_name,
        move |_window, cx| {
            let state = Rc::clone(&factory_state);
            let teardown_pending = Rc::clone(&factory_teardown_pending);
            let teardown_in_progress = Rc::clone(&factory_teardown_in_progress);
            let pointer_cancel_pending = Rc::clone(&factory_pointer_cancel_pending);
            cx.new(move |cx| {
                PumpEditor::new(
                    state,
                    teardown_pending,
                    teardown_in_progress,
                    pointer_cancel_pending,
                    cx,
                )
            })
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
    .with_pointer_cancel_callback(move || {
        pointer_cancel_pending.set(true);
        if let Ok(mut state) = pointer_cancel_state.try_borrow_mut() {
            state.cancel_active_gestures();
        }
    })
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
    pointer_cancel_pending: Rc<Cell<bool>>,
    curve_bounds: Rc<RefCell<Option<Bounds<Pixels>>>>,
    editor_focus_handle: FocusHandle,
    focus_out_subscription: Option<gpui::Subscription>,
    numeric_inputs: Vec<Entity<NumericInput>>,
    _numeric_subscriptions: Vec<gpui::Subscription>,
    curve_drag_node: Option<usize>,
    curve_drag_segment: Option<usize>,
    curve_active_button: Option<MouseButton>,
    curve_dragging_offset: bool,
    curve_drag_start: Option<Point<Pixels>>,
    curve_dragging_marquee: bool,
    curve_dragging_paint: bool,
    pending_option_gesture: Option<PendingOptionGesture>,
    pending_empty_node: Option<(Point<Pixels>, CurveNode)>,
    pending_seam: Option<(Point<Pixels>, bool)>,
    active_knob: Option<NumericEntryTarget>,
    last_pointer: Option<Point<Pixels>>,
    button_focus_handles: HashMap<&'static str, FocusHandle>,
    button_activation_keys: HashSet<String>,
}

#[derive(Clone, Copy)]
enum PendingOptionTarget {
    Node(usize),
    Segment(usize),
}

#[derive(Clone, Copy)]
struct PendingOptionGesture {
    origin: Point<Pixels>,
    target: PendingOptionTarget,
}

impl PumpEditor {
    fn new(
        state: Rc<RefCell<PumpEditorState>>,
        teardown_pending: Rc<Cell<bool>>,
        teardown_in_progress: Rc<Cell<bool>>,
        pointer_cancel_pending: Rc<Cell<bool>>,
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
                    let (target, draft) = input.update(cx, |input, _| {
                        input.set_editing(false);
                        (input.target, input.content().to_owned())
                    });
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
                    let target = input.update(cx, |input, _| {
                        input.set_editing(false);
                        input.target
                    });
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
        let button_focus_handles = [
            "timing-mode",
            "timing-value",
            "undo",
            "redo",
            "sound-a",
            "sound-switch",
            "sound-b",
            "hotkey-help",
            "waveform-mode",
            "bypass",
        ]
        .into_iter()
        .map(|id| (id, cx.focus_handle()))
        .collect();
        Self {
            state,
            teardown_pending,
            teardown_in_progress,
            pointer_cancel_pending,
            curve_bounds: Rc::new(RefCell::new(None)),
            editor_focus_handle: cx.focus_handle(),
            focus_out_subscription: None,
            numeric_inputs,
            _numeric_subscriptions: numeric_subscriptions,
            curve_drag_node: None,
            curve_drag_segment: None,
            curve_active_button: None,
            curve_dragging_offset: false,
            curve_drag_start: None,
            curve_dragging_marquee: false,
            curve_dragging_paint: false,
            pending_option_gesture: None,
            pending_empty_node: None,
            pending_seam: None,
            active_knob: None,
            last_pointer: None,
            button_focus_handles,
            button_activation_keys: HashSet::new(),
        }
    }

    fn button_focus_handle(&self, id: &'static str) -> &FocusHandle {
        self.button_focus_handles
            .get(id)
            .expect("all keyboard controls have a focus handle")
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
        let input = self.numeric_inputs[Self::numeric_input_index(target)].clone();
        let (_, formatted_text) = knob_value(&self.state.borrow(), target);
        let text = if target == NumericEntryTarget::Delay && input.read(cx).editing {
            self.state.borrow().params().delay_beats().to_string()
        } else {
            formatted_text
        };
        input.update(cx, |input, cx| input.set_content(text, select_all, cx));
    }

    fn sync_inactive_numeric_inputs(&self, cx: &mut Context<Self>) {
        let targets = [
            NumericEntryTarget::Mix,
            NumericEntryTarget::OutputGain,
            NumericEntryTarget::Smooth,
            NumericEntryTarget::Swing,
            NumericEntryTarget::FreeRate,
            NumericEntryTarget::Delay,
        ];
        let texts = {
            let state = self.state.borrow();
            targets
                .into_iter()
                .map(|target| (target, knob_value(&state, target).1))
                .collect::<Vec<_>>()
        };
        for (target, text) in texts {
            let input = self.numeric_inputs[Self::numeric_input_index(target)].clone();
            input.update(cx, |input, cx| {
                if !input.editing && input.content != text {
                    input.set_content(text, false, cx);
                }
            });
        }
    }

    fn begin_numeric_input(&mut self, target: NumericEntryTarget, cx: &mut Context<Self>) {
        self.dismiss_timing_dropdown(cx);
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
                if state.numeric_entry_active() {
                    drop(state);
                    self.dispatch(
                        EditorMessage::NumericEntry(NumericEntryMessage::Step { target, delta }),
                        cx,
                    );
                    self.sync_numeric_input(target, cx, false);
                    return;
                }
                let current_delay = state.params().delay_beats() as i32;
                let next_delay = (current_delay + delta).clamp(
                    crate::params::MIN_DELAY_BEATS as i32,
                    crate::params::MAX_DELAY_BEATS as i32,
                );
                let normalized = normalized_from_plain_value(PARAM_DELAY_ID, next_delay as f64)
                    .unwrap_or(current as f64) as f32;
                drop(state);
                self.dispatch(
                    EditorMessage::Knob {
                        target,
                        message: KnobMessage::Discrete { value: normalized },
                    },
                    cx,
                );
                self.sync_numeric_input(target, cx, true);
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

    fn clear_local_pointer_gesture(&mut self) {
        self.curve_drag_node = None;
        self.curve_drag_segment = None;
        self.curve_active_button = None;
        self.curve_dragging_offset = false;
        self.curve_drag_start = None;
        self.curve_dragging_marquee = false;
        self.curve_dragging_paint = false;
        self.pending_option_gesture = None;
        self.pending_empty_node = None;
        self.pending_seam = None;
        self.active_knob = None;
        self.last_pointer = None;
    }

    fn consume_pointer_cancel(&mut self, cx: &mut Context<Self>) {
        if !self.pointer_cancel_pending.replace(false) {
            return;
        }

        self.clear_local_pointer_gesture();
        if let Ok(mut state) = self.state.try_borrow_mut() {
            state.cancel_active_gestures();
        } else {
            // The callback can run while GPUI is dispatching another editor
            // event. Keep the marker set so the next frame retries the model
            // cancellation after that borrow has ended.
            self.pointer_cancel_pending.set(true);
        }
        self.clear_curve_hover(cx);
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

    fn insertion_node_on_curve(&self, position: Point<Pixels>) -> Option<CurveNode> {
        let mut node = self.raw_curve_node(position)?;
        node.y = sample_editable_curve(&self.state.borrow().rendered_curve(), node.x);
        Some(node)
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

    fn curve_plot_contains(&self, position: Point<Pixels>) -> bool {
        let Some(bounds) = self.curve_bounds.borrow().as_ref().copied() else {
            return false;
        };
        let left = f32::from(bounds.left()) + CURVE_GUTTER;
        let top = f32::from(bounds.top());
        let width =
            (f32::from(bounds.size.width) - CURVE_GUTTER - CURVE_METER_GAP - CURVE_METER_WIDTH)
                .max(1.0);
        let height =
            (f32::from(bounds.size.height) - CURVE_OFFSET_BAR_HEIGHT - CURVE_OFFSET_INSET).max(1.0);
        let x = f32::from(position.x);
        let y = f32::from(position.y);
        x >= left && x <= left + width && y >= top && y <= top + height
    }

    fn curve_paint_sample_with_outside(
        &self,
        position: Point<Pixels>,
        outside: bool,
    ) -> Option<CurvePaintSample> {
        let display = self.normalized_curve_point(position)?;
        Some(CurvePaintSample {
            node: self.raw_curve_node(position)?,
            display_position: super::curve_paint::RectPoint {
                x: display.x,
                y: display.y,
            },
            outside,
        })
    }

    fn dispatch_curve_hover(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        self.last_pointer = Some(event.position);
        let command = event.modifiers.platform || event.modifiers.control;
        let changed = {
            let state = self.state.borrow();
            state.option_hover_held() != event.modifiers.alt
                || state.command_hover_held() != command
                || state.shift_hover_held() != event.modifiers.shift
        };
        if changed {
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                    option_held: event.modifiers.alt,
                    command_held: command,
                    shift_held: event.modifiers.shift,
                }),
                cx,
            );
        }
        if self.seam_at(event.position).is_some() {
            self.clear_curve_hover(cx);
            return;
        }
        let node = self.node_at(event.position);
        let segment = if node.is_none() {
            self.segment_at(event.position)
        } else {
            None
        };
        let command = event.modifiers.platform || event.modifiers.control;
        let option = event.modifiers.alt;
        let preview_node = if !command && !option {
            segment
                .and_then(|(_, distance)| {
                    (distance <= 4.0).then(|| self.insertion_node_on_curve(event.position))
                })
                .flatten()
        } else {
            None
        };
        if let Some((index, distance)) = segment {
            if distance > 8.0 && !command && !option {
                self.dispatch(
                    EditorMessage::Curve(CurvePreviewMessage::HoverProximitySegment { index }),
                    cx,
                );
                return;
            }
        }
        self.dispatch(
            EditorMessage::Curve(CurvePreviewMessage::Hover {
                node,
                preview_node,
                segment: segment.map(|(index, _)| index),
            }),
            cx,
        );
    }

    fn clear_curve_hover(&mut self, cx: &mut Context<Self>) {
        let should_clear = {
            let state = self.state.borrow();
            state.hover_node().is_some()
                || state.preview_node().is_some()
                || state.hover_segment().is_some()
        };
        if should_clear {
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::Hover {
                    node: None,
                    preview_node: None,
                    segment: None,
                }),
                cx,
            );
        }
    }

    fn option_gesture_drag_started(origin: Point<Pixels>, position: Point<Pixels>) -> bool {
        let dx = f32::from(position.x - origin.x);
        let dy = f32::from(position.y - origin.y);
        dx * dx + dy * dy >= OPTION_GESTURE_DRAG_START_DISTANCE_SQUARED
    }

    fn pending_option_handoff(&mut self, pending: PendingOptionGesture, cx: &mut Context<Self>) {
        match pending.target {
            PendingOptionTarget::Node(index) => {
                let pointer = self
                    .raw_curve_node(pending.origin)
                    .unwrap_or(CurveNode { x: 0.0, y: 0.0 });
                self.dispatch(
                    EditorMessage::Curve(CurvePreviewMessage::PressNode {
                        index,
                        pointer,
                        shift_held: false,
                        option_held: true,
                        command_held: false,
                    }),
                    cx,
                );
                self.curve_drag_node.replace(index);
            }
            PendingOptionTarget::Segment(index) => {
                self.dispatch(
                    EditorMessage::Curve(CurvePreviewMessage::PressSegment {
                        index,
                        position: ModelPoint::new(
                            f32::from(pending.origin.x),
                            f32::from(pending.origin.y),
                        ),
                    }),
                    cx,
                );
                self.curve_drag_segment.replace(index);
            }
        }
    }

    fn seam_at(&self, position: Point<Pixels>) -> Option<bool> {
        let bounds = self.curve_bounds.borrow().as_ref().copied()?;
        let dimensions = self.curve_dimensions()?;
        let state = self.state.borrow();
        let y = super::projection::sample_display_curve(
            &state.rendered_curve(),
            0.0,
            state.params().phase_offset(),
        );
        let top = f32::from(bounds.top());
        let left = f32::from(bounds.left()) + CURVE_GUTTER;
        let py = top + (1.0 - y) * (dimensions.y - 1.0).max(1.0);
        [false, true].into_iter().find(|right| {
            let px = left
                + if *right {
                    (dimensions.x - 1.0).max(1.0)
                } else {
                    0.0
                };
            (f32::from(position.x) - px).hypot(f32::from(position.y) - py) <= CURVE_NODE_HIT_RADIUS
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
        let seam_indices = state.seam_node_indices();
        for (index, node) in curve.nodes.iter().copied().enumerate() {
            if seam_indices.contains(&index) {
                continue;
            }
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
        let mut nearest = None;
        let mut nearest_distance = f32::INFINITY;
        // Hit-test the rendered shape, not the straight chord between nodes.
        for index in 0..curve.nodes.len().saturating_sub(1) {
            for polyline in
                sampled_curve_segment_polylines(&curve, index, left, top, width, height, phase)
            {
                for pair in polyline.windows(2) {
                    let sx = f32::from(pair[0].x);
                    let sy = f32::from(pair[0].y);
                    let dx = f32::from(pair[1].x) - sx;
                    let dy = f32::from(pair[1].y) - sy;
                    let length_sq = dx * dx + dy * dy;
                    let t = if length_sq <= f32::EPSILON {
                        0.0
                    } else {
                        (((f32::from(position.x) - sx) * dx + (f32::from(position.y) - sy) * dy)
                            / length_sq)
                            .clamp(0.0, 1.0)
                    };
                    let distance = ((f32::from(position.x) - sx - t * dx).powi(2)
                        + (f32::from(position.y) - sy - t * dy).powi(2))
                    .sqrt();
                    if distance < nearest_distance {
                        nearest_distance = distance;
                        nearest = Some(index);
                    }
                }
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
        self.dismiss_timing_dropdown(cx);
        self.last_pointer = Some(event.position);
        self.curve_drag_start = Some(event.position);
        self.curve_drag_segment = None;
        self.curve_drag_node = None;
        self.curve_active_button = None;
        self.curve_dragging_offset = false;
        self.curve_dragging_marquee = false;
        self.curve_dragging_paint = false;
        self.pending_option_gesture = None;
        self.pending_empty_node = None;
        self.pending_seam = None;
        let display_point = self
            .normalized_curve_point(event.position)
            .unwrap_or_default();
        let point = self
            .raw_curve_node(event.position)
            .unwrap_or(CurveNode { x: 0.0, y: 0.0 });
        let shift = event.modifiers.shift;
        let option = event.modifiers.alt;
        let command = event.modifiers.platform || event.modifiers.control;

        // Secondary-button input is always freehand paint inside the plot.
        // Keep it ahead of node/segment admission so a right drag over an
        // existing element cannot enter a primary gesture branch.
        if event.button == MouseButton::Right {
            if self.curve_plot_contains(event.position) {
                self.curve_active_button = Some(MouseButton::Right);
                self.curve_dragging_paint = true;
                self.dispatch(
                    EditorMessage::Curve(CurvePreviewMessage::PressPaint {
                        sample: self.curve_paint_sample(event.position).unwrap_or(
                            CurvePaintSample {
                                node: point,
                                display_position: super::curve_paint::RectPoint {
                                    x: display_point.x,
                                    y: display_point.y,
                                },
                                outside: false,
                            },
                        ),
                    }),
                    cx,
                );
            }
            return;
        }

        if !(command && shift) {
            if let Some(right_edge) = self.seam_at(event.position) {
                self.curve_active_button = Some(MouseButton::Left);
                self.pending_seam = Some((event.position, right_edge));
                return;
            }
        }

        if event.button == MouseButton::Left && event.click_count >= 2 {
            if self.in_curve_offset_bar(event.position) {
                self.dispatch(
                    EditorMessage::Curve(CurvePreviewMessage::ResetCurveOffset),
                    cx,
                );
                return;
            }
            if let Some(index) = self.node_at(event.position) {
                let deletable = {
                    let state = self.state.borrow();
                    let node_count = state.rendered_curve().nodes.len();
                    index > 0 && index + 1 < node_count
                };
                if deletable {
                    self.dispatch(
                        EditorMessage::Curve(CurvePreviewMessage::DeleteNode { index }),
                        cx,
                    );
                    return;
                }
            }
        }

        // Option-click is a deferred gesture: a release on a deletable node
        // removes it, while a real drag is handed off to the established
        // constrained node/segment operation. This avoids deleting a node
        // when the user intended to adjust it.
        if option && !command && !shift && self.curve_plot_contains(event.position) {
            let target = self
                .node_at(event.position)
                .map(PendingOptionTarget::Node)
                .or_else(|| {
                    self.segment_at(event.position)
                        .filter(|(_, distance)| *distance <= 7.0)
                        .map(|(index, _)| PendingOptionTarget::Segment(index))
                });
            if let Some(target) = target {
                self.pending_option_gesture = Some(PendingOptionGesture {
                    origin: event.position,
                    target,
                });
                self.curve_active_button = Some(MouseButton::Left);
                return;
            }
        }

        if self.in_curve_offset_bar(event.position)
            || (event.button == MouseButton::Left && command && shift)
        {
            if let Some(pointer_x) = self.curve_offset_pointer_x(event.position) {
                self.curve_active_button = Some(MouseButton::Left);
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
            self.curve_active_button = Some(MouseButton::Left);
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
            self.curve_active_button = Some(MouseButton::Left);
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
            self.curve_active_button = Some(MouseButton::Left);
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
            self.curve_active_button = Some(MouseButton::Left);
            self.curve_drag_segment = Some(index);
            self.curve_dragging_paint = false;
            let position =
                ModelPoint::new(f32::from(event.position.x), f32::from(event.position.y));
            let message = if option {
                CurvePreviewMessage::PressSegment { index, position }
            } else if distance <= 4.0 {
                self.curve_drag_segment = None;
                CurvePreviewMessage::InsertNode {
                    node: self
                        .insertion_node_on_curve(event.position)
                        .unwrap_or(point),
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
            if !option && self.curve_plot_contains(event.position) {
                self.curve_active_button = Some(MouseButton::Left);
                self.pending_empty_node = Some((event.position, point));
            }
        }
    }

    fn curve_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.curve_active_button.is_none() {
            if event.pressed_button.is_some() {
                return;
            }
            if event.pressed_button.is_none() {
                self.dispatch_curve_hover(event, cx);
            }
            return;
        }
        let Some(display_point) = self.normalized_curve_point(event.position) else {
            return;
        };
        let point = self
            .raw_curve_node(event.position)
            .unwrap_or(CurveNode { x: 0.0, y: 0.0 });
        self.last_pointer = Some(event.position);
        if let Some((origin, right_edge)) = self.pending_seam {
            if f32::from(event.position.y - origin.y).abs() < 3.0 {
                return;
            }
            self.pending_seam = None;
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::PressSeam { right_edge }),
                cx,
            );
            self.curve_drag_node = self.state.borrow().active_node();
        }
        if let Some((origin, node)) = self.pending_empty_node {
            if (f32::from(event.position.x - origin.x))
                .hypot(f32::from(event.position.y - origin.y))
                < 3.0
            {
                return;
            }
            self.pending_empty_node = None;
            self.dispatch(
                EditorMessage::Curve(CurvePreviewMessage::InsertNode {
                    node,
                    command_held: false,
                }),
                cx,
            );
            self.curve_drag_node = self.state.borrow().active_node();
        }
        if let Some(pending) = self.pending_option_gesture {
            if !event.modifiers.alt
                || event.modifiers.platform
                || event.modifiers.control
                || event.modifiers.shift
            {
                self.pending_option_gesture = None;
                self.curve_active_button = None;
                return;
            }
            if !Self::option_gesture_drag_started(pending.origin, event.position) {
                return;
            }
            self.pending_option_gesture = None;
            self.pending_option_handoff(pending, cx);
        }
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
            let delta = (f32::from(start.x) - f32::from(event.position.x)) / dimensions.x.max(1.0);
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
            let outside = !self.curve_plot_contains(event.position);
            let sample = self
                .curve_paint_sample_with_outside(event.position, outside)
                .unwrap_or(CurvePaintSample {
                    node: point,
                    display_position: super::curve_paint::RectPoint {
                        x: display_point.x,
                        y: display_point.y,
                    },
                    outside,
                });
            self.dispatch(
                EditorMessage::Curve(if outside {
                    CurvePreviewMessage::DragPaintOutside { sample }
                } else {
                    CurvePreviewMessage::DragPaint { sample }
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
        if self.curve_active_button != Some(event.button) {
            return;
        }
        let display_point = self
            .normalized_curve_point(event.position)
            .unwrap_or_default();
        let point = self
            .raw_curve_node(event.position)
            .unwrap_or(CurveNode { x: 0.0, y: 0.0 });
        self.pending_empty_node = None;
        self.pending_seam = None;
        if let Some(pending) = self.pending_option_gesture.take() {
            if let PendingOptionTarget::Node(index) = pending.target {
                let deletable = {
                    let state = self.state.borrow();
                    let node_count = state.rendered_curve().nodes.len();
                    index > 0 && index + 1 < node_count
                };
                if event.modifiers.alt
                    && !event.modifiers.platform
                    && !event.modifiers.control
                    && !event.modifiers.shift
                    && !Self::option_gesture_drag_started(pending.origin, event.position)
                    && self.node_at(event.position) == Some(index)
                    && deletable
                {
                    self.dispatch(
                        EditorMessage::Curve(CurvePreviewMessage::DeleteNode { index }),
                        cx,
                    );
                }
            }
        } else if self.curve_dragging_marquee {
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
            let delta = (f32::from(start.x) - f32::from(event.position.x)) / dimensions.x.max(1.0);
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
            let outside = !self.curve_plot_contains(event.position);
            let sample = self
                .curve_paint_sample_with_outside(event.position, outside)
                .unwrap_or(CurvePaintSample {
                    node: point,
                    display_position: super::curve_paint::RectPoint {
                        x: display_point.x,
                        y: display_point.y,
                    },
                    outside,
                });
            let message = if outside {
                CurvePreviewMessage::ReleasePaintOutside { sample }
            } else {
                CurvePreviewMessage::ReleasePaint {
                    sample: Some(sample),
                }
            };
            self.dispatch(EditorMessage::Curve(message), cx);
        }
        self.curve_drag_start = None;
        self.last_pointer = None;
        self.curve_active_button = None;
        self.pending_option_gesture = None;
    }

    fn captured_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.consume_pointer_cancel(cx);
        if self.active_knob.is_some() {
            self.knob_move(event, window, cx);
            return true;
        }
        if self.curve_active_button.is_some() {
            self.curve_mouse_move(event, window, cx);
            return true;
        }
        if self.state.borrow().timing_dropdown_open() {
            self.clear_curve_hover(cx);
            return false;
        }
        if self
            .curve_bounds
            .borrow()
            .as_ref()
            .is_some_and(|bounds| bounds.contains(&event.position))
        {
            self.dispatch_curve_hover(event, cx);
        } else {
            self.clear_curve_hover(cx);
        }
        false
    }

    fn captured_mouse_down(&mut self, cx: &mut Context<Self>) -> bool {
        self.consume_pointer_cancel(cx);
        self.active_knob.is_some() || self.curve_active_button.is_some()
    }

    fn captured_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.consume_pointer_cancel(cx);
        if self.active_knob.is_some() {
            if event.button == MouseButton::Left {
                self.knob_up(event, window, cx);
            }
            return true;
        }
        if self.curve_active_button == Some(event.button) {
            self.curve_mouse_up(event, window, cx);
            return true;
        }
        self.curve_active_button.is_some()
    }

    fn captured_mouse_exit(&mut self, cx: &mut Context<Self>) {
        if self.active_knob.is_none() && self.curve_active_button.is_none() {
            self.clear_curve_hover(cx);
            self.last_pointer = None;
        }
    }

    fn knob_down(
        &mut self,
        target: NumericEntryTarget,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.numeric_inputs[Self::numeric_input_index(target)].clone();
        let focus_handle = input.read(cx).focus_handle.clone();
        window.focus(&focus_handle, cx);
        self.dismiss_timing_dropdown(cx);
        if event.click_count >= 2 {
            self.dispatch(
                EditorMessage::Knob {
                    target,
                    message: KnobMessage::Reset {
                        value: PumpEditorState::default_knob_normalized(target),
                    },
                },
                cx,
            );
            self.active_knob = None;
            self.last_pointer = None;
            return;
        }
        self.active_knob = Some(target);
        self.last_pointer = Some(event.position);
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

    fn knob_wheel(
        &mut self,
        target: NumericEntryTarget,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.numeric_inputs[Self::numeric_input_index(target)].clone();
        let focus_handle = input.read(cx).focus_handle.clone();
        window.focus(&focus_handle, cx);
        let delta = f32::from(event.delta.pixel_delta(px(16.0)).y);
        if delta.abs() <= f32::EPSILON {
            return;
        }
        let multiplier = if event.modifiers.shift { 4.0 } else { 1.0 };
        let direction = delta.signum() * multiplier;
        let current = self.state.borrow();
        let (normalized, _) = knob_value(&current, target);
        drop(current);
        let step = match target {
            NumericEntryTarget::OutputGain => {
                1.0 / (crate::params::MAX_OUTPUT_GAIN_DB - crate::params::MIN_OUTPUT_GAIN_DB)
            }
            NumericEntryTarget::FreeRate => 0.01,
            NumericEntryTarget::Delay => {
                let current = self.state.borrow().params().delay_beats() as i32;
                let next = (current + direction as i32).clamp(
                    crate::params::MIN_DELAY_BEATS as i32,
                    crate::params::MAX_DELAY_BEATS as i32,
                ) as f32;
                normalized_from_plain_value(PARAM_DELAY_ID, next as f64)
                    .unwrap_or(normalized as f64) as f32
                    - normalized
            }
            _ => 0.01,
        };
        self.dispatch(
            EditorMessage::Knob {
                target,
                message: KnobMessage::Discrete {
                    value: if target == NumericEntryTarget::Delay {
                        (normalized + step).clamp(0.0, 1.0)
                    } else {
                        (normalized + direction * step).clamp(0.0, 1.0)
                    },
                },
            },
            cx,
        );
        self.sync_numeric_input(target, cx, false);
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

    fn dismiss_timing_dropdown(&mut self, cx: &mut Context<Self>) {
        if self.state.borrow().timing_dropdown_open() {
            self.dispatch(EditorMessage::ToggleTimingDropdown, cx);
        }
        if self.state.borrow().hotkey_help_open() {
            self.dispatch(EditorMessage::ToggleHotkeyHelp, cx);
        }
    }

    fn toggle_bypass(
        &mut self,
        event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A numeric field may still own focus when the pointer leaves it. The
        // focus transition cancels its draft; claim the button handle here as
        // well as on mouse-down so the following Space/Enter is routed to the
        // focused bypass control even when the click lands on a child icon.
        let focus_handle = self.button_focus_handle("bypass").clone();
        window.focus(&focus_handle, cx);
        if event.is_keyboard() {
            return;
        }
        self.dismiss_timing_dropdown(cx);
        self.dispatch(EditorMessage::ToggleBypass, cx);
    }

    fn toggle_timing(
        &mut self,
        event: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.is_keyboard() {
            return;
        }
        self.dismiss_timing_dropdown(cx);
        self.dispatch(EditorMessage::ToggleTimingMode, cx);
    }

    fn select_sound_a(
        &mut self,
        event: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.is_keyboard() {
            return;
        }
        self.dismiss_timing_dropdown(cx);
        let active = self.state.borrow().params().active_sound();
        self.dispatch(
            EditorMessage::SelectSound {
                side: SoundSide::A,
                copy: event.modifiers().alt && active != SoundSide::A,
            },
            cx,
        );
    }

    fn select_sound_b(
        &mut self,
        event: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.is_keyboard() {
            return;
        }
        self.dismiss_timing_dropdown(cx);
        let active = self.state.borrow().params().active_sound();
        self.dispatch(
            EditorMessage::SelectSound {
                side: SoundSide::B,
                copy: event.modifiers().alt && active != SoundSide::B,
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
        if event.is_keyboard() {
            return;
        }
        self.dismiss_timing_dropdown(cx);
        let active = self.state.borrow().params().active_sound();
        if event.modifiers().platform || event.modifiers().control {
            self.dispatch(EditorMessage::CopyAndSelectSound(active.other()), cx);
        } else {
            self.dispatch(
                EditorMessage::SelectSound {
                    side: active.other(),
                    copy: false,
                },
                cx,
            );
        }
    }

    fn undo(&mut self, event: &gpui::ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.is_keyboard() {
            return;
        }
        self.dismiss_timing_dropdown(cx);
        self.dispatch(EditorMessage::Undo, cx);
    }

    fn redo(&mut self, event: &gpui::ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.is_keyboard() {
            return;
        }
        self.dismiss_timing_dropdown(cx);
        self.dispatch(EditorMessage::Redo, cx);
    }

    fn toggle_timing_dropdown(
        &mut self,
        event: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.is_keyboard() {
            return;
        }
        if self.state.borrow().hotkey_help_open() {
            self.dispatch(EditorMessage::ToggleHotkeyHelp, cx);
        }
        self.dispatch(EditorMessage::ToggleTimingDropdown, cx);
    }

    fn select_sync_division(
        &mut self,
        index: usize,
        _: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(
            EditorMessage::SyncDivision(PumpEditorState::normalized_sync_division(index)),
            cx,
        );
        cx.stop_propagation();
    }

    fn select_free_rate_unit(
        &mut self,
        unit: FreeRateUnit,
        _: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch(EditorMessage::FreeRateUnit(unit), cx);
        cx.stop_propagation();
    }

    fn toggle_hotkey_help(
        &mut self,
        event: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.is_keyboard() {
            return;
        }
        self.dispatch(EditorMessage::ToggleHotkeyHelp, cx);
    }

    fn toggle_waveform(
        &mut self,
        event: &gpui::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.is_keyboard() {
            return;
        }
        self.dismiss_timing_dropdown(cx);
        self.dispatch(EditorMessage::ToggleWaveformMode, cx);
    }

    fn slot_click(
        &mut self,
        index: usize,
        event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Slots are transport controls, not text fields. Move focus back to
        // the editor root before dispatching the load/store so a previously
        // focused numeric input cannot consume the next keyboard command.
        window.focus(&self.editor_focus_handle, cx);
        self.dismiss_timing_dropdown(cx);
        let message = if event.modifiers().platform || event.modifiers().control {
            super::model::CurveSlotMessage::Store { index }
        } else {
            super::model::CurveSlotMessage::Load { index }
        };
        self.dispatch(EditorMessage::CurveSlot(message), cx);
    }

    #[allow(dead_code)]
    fn step_delay(&mut self, delta: i32, _window: &mut Window, cx: &mut Context<Self>) {
        self.step_numeric_input(NumericEntryTarget::Delay, delta, cx);
    }

    fn handle_modifiers(
        &mut self,
        event: &ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.modifiers.alt && self.pending_option_gesture.is_some() {
            self.pending_option_gesture = None;
            self.curve_active_button = None;
        }
        self.dispatch(
            EditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                option_held: event.modifiers.alt,
                command_held: event.modifiers.platform || event.modifiers.control,
                shift_held: event.modifiers.shift,
            }),
            cx,
        );
    }

    fn focused_button(&self, window: &Window) -> Option<&'static str> {
        [
            "timing-mode",
            "timing-value",
            "undo",
            "redo",
            "sound-a",
            "sound-switch",
            "sound-b",
            "hotkey-help",
            "waveform-mode",
            "bypass",
        ]
        .into_iter()
        .find(|id| {
            self.button_focus_handles
                .get(id)
                .is_some_and(|handle| handle.is_focused(window))
        })
    }

    fn activate_focused_button(&mut self, id: &'static str, cx: &mut Context<Self>) {
        match id {
            "timing-mode" => {
                self.dismiss_timing_dropdown(cx);
                self.dispatch(EditorMessage::ToggleTimingMode, cx);
            }
            "timing-value" => {
                if self.state.borrow().hotkey_help_open() {
                    self.dispatch(EditorMessage::ToggleHotkeyHelp, cx);
                }
                self.dispatch(EditorMessage::ToggleTimingDropdown, cx);
            }
            "undo" => {
                self.dismiss_timing_dropdown(cx);
                self.dispatch(EditorMessage::Undo, cx);
            }
            "redo" => {
                self.dismiss_timing_dropdown(cx);
                self.dispatch(EditorMessage::Redo, cx);
            }
            "sound-a" => {
                self.dismiss_timing_dropdown(cx);
                self.dispatch(
                    EditorMessage::SelectSound {
                        side: SoundSide::A,
                        copy: false,
                    },
                    cx,
                );
            }
            "sound-switch" => {
                self.dismiss_timing_dropdown(cx);
                let side = self.state.borrow().params().active_sound().other();
                self.dispatch(EditorMessage::SelectSound { side, copy: false }, cx);
            }
            "sound-b" => {
                self.dismiss_timing_dropdown(cx);
                self.dispatch(
                    EditorMessage::SelectSound {
                        side: SoundSide::B,
                        copy: false,
                    },
                    cx,
                );
            }
            "hotkey-help" => self.dispatch(EditorMessage::ToggleHotkeyHelp, cx),
            "waveform-mode" => {
                self.dismiss_timing_dropdown(cx);
                self.dispatch(EditorMessage::ToggleWaveformMode, cx);
            }
            "bypass" => {
                self.dismiss_timing_dropdown(cx);
                self.dispatch(EditorMessage::ToggleBypass, cx);
            }
            _ => {}
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let numeric_input_focused = self
            .numeric_inputs
            .iter()
            .any(|input| input.read(cx).focus_handle.is_focused(window));
        if self.state.borrow().numeric_entry_active() || numeric_input_focused {
            return;
        }
        if matches!(event.keystroke.key.as_str(), "space" | "enter")
            && !event.keystroke.modifiers.modified()
        {
            if let Some(button) = self.focused_button(window) {
                // Native embedded GPUI input currently loses AppKit's
                // `isARepeat` flag: every repeated key-down arrives with
                // `is_held == false`. Keep the press state at the editor
                // boundary and release it on the matching key-up, so both
                // native and direct GPUI paths activate focused buttons once.
                let key = event.keystroke.key.to_string();
                if event.is_held || !self.button_activation_keys.insert(key) {
                    window.prevent_default();
                    cx.stop_propagation();
                    return;
                }
                self.activate_focused_button(button, cx);
                window.prevent_default();
                cx.stop_propagation();
                return;
            }
        }
        match event.keystroke.key.as_str() {
            "escape" => {
                if self.state.borrow().timing_dropdown_open() {
                    self.dispatch(EditorMessage::ToggleTimingDropdown, cx);
                    return;
                }
                if self.state.borrow().hotkey_help_open() {
                    self.dispatch(EditorMessage::ToggleHotkeyHelp, cx);
                    return;
                }
                if self.state.borrow().has_active_gesture() {
                    self.state.borrow_mut().cancel_active_gestures();
                    self.drain_teardown_pending();
                    cx.notify();
                }
            }
            "enter" if self.state.borrow().timing_dropdown_open() => {
                self.dispatch(EditorMessage::ToggleTimingDropdown, cx);
            }
            "up" | "down" if self.state.borrow().timing_dropdown_open() => {
                let direction = if event.keystroke.key == "down" {
                    1_i32
                } else {
                    -1_i32
                };
                let state = self.state.borrow();
                if state.params().timing_mode() == TIMING_MODE_FREE {
                    let current = FreeRateUnit::ALL
                        .iter()
                        .position(|unit| *unit == state.free_rate_unit())
                        .unwrap_or(0) as i32;
                    let next =
                        (current + direction).rem_euclid(FreeRateUnit::ALL.len() as i32) as usize;
                    drop(state);
                    self.dispatch(EditorMessage::FreeRateUnit(FreeRateUnit::ALL[next]), cx);
                } else {
                    let current = state.params().sync_division() as i32;
                    let next = (current + direction)
                        .clamp(0, SYNC_DIVISIONS.len().saturating_sub(1) as i32)
                        as usize;
                    drop(state);
                    self.dispatch(
                        EditorMessage::SyncDivision(PumpEditorState::normalized_sync_division(
                            next,
                        )),
                        cx,
                    );
                }
            }
            "delete" | "backspace" => {
                self.dispatch(
                    EditorMessage::Curve(CurvePreviewMessage::DeleteSelectedNodes),
                    cx,
                );
            }
            _ => {}
        }
    }

    fn handle_key_up(&mut self, event: &KeyUpEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        self.button_activation_keys
            .remove(event.keystroke.key.as_str());
    }

    fn pump_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.consume_pointer_cancel(cx);
        self.drain_teardown_pending();
        self.state.borrow_mut().refresh_host_projection();
        self.sync_inactive_numeric_inputs(cx);
        // Host parameter callbacks do not necessarily invalidate this GPUI
        // entity. Poll the lock-free projection on every native frame so idle
        // A/B automation, meters, and waveform state become visible without
        // touching the audio thread.
        window.request_animation_frame();
    }
}

impl Drop for PumpEditor {
    fn drop(&mut self) {
        self.button_activation_keys.clear();
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

#[allow(clippy::too_many_arguments)]
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

fn curve_point_pixels(
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    phase: f32,
    node: CurveNode,
) -> Point<Pixels> {
    let display_x = (node.x - phase).rem_euclid(1.0);
    point(
        px(left + display_x * (width - 1.0).max(1.0)),
        px(top + (1.0 - node.y.clamp(0.0, 1.0)) * (height - 1.0).max(1.0)),
    )
}

fn sampled_curve_segment_polylines(
    curve: &crate::curve::EditableCurve,
    index: usize,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    phase: f32,
) -> Vec<Vec<Point<Pixels>>> {
    let (Some(start), Some(end)) = (
        curve.nodes.get(index).copied(),
        curve.nodes.get(index + 1).copied(),
    ) else {
        return Vec::new();
    };
    let span = (end.x - start.x).max(0.0);
    let steps = (span * (width - 1.0).max(1.0)).ceil().clamp(2.0, 128.0) as usize;
    let mut polylines: Vec<Vec<Point<Pixels>>> = vec![Vec::with_capacity(steps + 1)];
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let x = start.x + (end.x - start.x) * t;
        let y = sample_editable_curve(curve, x).clamp(0.0, 1.0);
        let current = curve_point_pixels(left, top, width, height, phase, CurveNode { x, y });
        let previous_x: Option<f32> = polylines
            .last()
            .and_then(|polyline| polyline.last())
            .map(|point| f32::from(point.x));
        if previous_x.is_some_and(|previous| f32::from(current.x) + 1.0e-5 < previous) {
            polylines.push(vec![current]);
        } else {
            polylines
                .last_mut()
                .expect("segment polyline exists")
                .push(current);
        }
    }
    polylines
}

fn draw_curve(
    bounds: Bounds<Pixels>,
    state: &PumpEditorState,
    last_pointer: Option<Point<Pixels>>,
    window: &mut Window,
    cx: &mut App,
) {
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
    let active_offset = state.active_curve_offset();
    let modifier_offset_hover = state.command_hover_held()
        && state.shift_hover_held()
        && last_pointer.is_some_and(|pointer| bounds.contains(&pointer));
    let mut area = gpui::PathBuilder::fill();
    let mut path = gpui::PathBuilder::stroke(px(if active_offset {
        2.55
    } else if modifier_offset_hover {
        2.975
    } else {
        CURVE_STROKE_WIDTH
    }));
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
    // The authored fill fades into the editor surface toward the lower edge,
    // matching the legacy visualization without introducing a renderer-owned
    // gradient abstraction.
    const FILL_FADE_STRIPES: usize = 12;
    for index in 0..FILL_FADE_STRIPES {
        let start = index as f32 / FILL_FADE_STRIPES as f32;
        let end = (index + 1) as f32 / FILL_FADE_STRIPES as f32;
        let alpha = (start * start * 100.0).round() as u8;
        if alpha == 0 {
            continue;
        }
        window.paint_quad(fill(
            Bounds::from_corners(
                point(px(left), px(top + height * start)),
                point(px(left + width), px(top + height * end)),
            ),
            solid(theme.clear.with_alpha(alpha)),
        ));
    }
    let curve_color = if active_offset {
        CURVE_OFFSET_MOVE_COLOR
    } else if modifier_offset_hover {
        CURVE_OFFSET_HOVER_COLOR
    } else {
        theme.accent_mint
    };
    if let Ok(path) = path.build() {
        window.paint_path(path, solid(curve_color));
    }
    let move_segment = state
        .active_segment()
        .filter(|_| state.active_segment_is_move())
        .or_else(|| {
            state
                .command_hover_held()
                .then_some(state.hover_segment())
                .flatten()
        })
        .or_else(|| {
            (!state.active_segment_is_move()
                && !state.option_hover_held()
                && !state.command_hover_held()
                && state.hover_segment_is_proximity())
            .then_some(state.hover_segment())
            .flatten()
        });
    let tension_segment = (!state.active_segment_is_move())
        .then_some(state.active_segment())
        .flatten()
        .or_else(|| {
            (!state.command_hover_held() && state.option_hover_held())
                .then_some(state.hover_segment())
                .flatten()
        });
    if let Some(segment) = move_segment.or(tension_segment) {
        let color = if move_segment == Some(segment) {
            CURVE_SEGMENT_MOVE_COLOR
        } else {
            theme.accent_warning
        };
        for points in
            sampled_curve_segment_polylines(&curve, segment, left, top, width, height, phase)
        {
            if points.len() < 2 {
                continue;
            }
            let mut highlighted = gpui::PathBuilder::stroke(px(2.975));
            highlighted.move_to(points[0]);
            for point in points.into_iter().skip(1) {
                highlighted.line_to(point);
            }
            if let Ok(path) = highlighted.build() {
                window.paint_path(path, solid(color));
            }
        }
    }
    if let Some(preview) = state.preview_node() {
        let center = curve_point_pixels(left, top, width, height, phase, preview);
        let mut ring = gpui::PathBuilder::stroke(px(1.5));
        for step in 0..=32 {
            let angle = std::f32::consts::TAU * step as f32 / 32.0;
            let point = point(
                center.x + px(5.0 * angle.cos()),
                center.y + px(5.0 * angle.sin()),
            );
            if step == 0 {
                ring.move_to(point);
            } else {
                ring.line_to(point);
            }
        }
        ring.close();
        if let Ok(path) = ring.build() {
            window.paint_path(path, solid(theme.accent_mint));
        }
    }
    for run in state.curve_paint_runs().unwrap_or_default() {
        let points: Vec<_> = run
            .points()
            .iter()
            .map(|sample| {
                point(
                    px(left + sample.position.x * (width - 1.0).max(1.0)),
                    px(top + (1.0 - sample.position.y) * (height - 1.0).max(1.0)),
                )
            })
            .collect();
        if points.len() < 2 {
            continue;
        }
        let mut paint_preview = gpui::PathBuilder::stroke(px(CURVE_PAINT_PREVIEW_WIDTH));
        paint_preview.move_to(points[0]);
        for point in points.into_iter().skip(1) {
            paint_preview.line_to(point);
        }
        if let Ok(path) = paint_preview.build() {
            window.paint_path(path, solid(theme.accent_copper));
        }
    }
    // A seam is one logical point represented at both clipping boundaries.
    // Sample it every frame; offset changes never materialize authored nodes.
    let seam_indices = state.seam_node_indices();
    let seam_y = top
        + (1.0 - super::projection::sample_display_curve(&curve, 0.0, phase))
            * (height - 1.0).max(1.0);
    let seam_active = state
        .active_node()
        .is_some_and(|index| seam_indices.contains(&index));
    for x in [left, left + (width - 1.0).max(1.0)] {
        let mut diamond = gpui::PathBuilder::stroke(px(if seam_active { 2.0 } else { 1.5 }));
        diamond.move_to(point(px(x), px(seam_y - 6.0)));
        diamond.line_to(point(px(x + 4.5), px(seam_y)));
        diamond.line_to(point(px(x), px(seam_y + 6.0)));
        diamond.line_to(point(px(x - 4.5), px(seam_y)));
        diamond.close();
        if let Ok(path) = diamond.build() {
            window.paint_path(
                path,
                solid(if seam_active {
                    theme.accent_warning
                } else {
                    theme.accent_mint
                }),
            );
        }
    }
    for (index, node) in curve.nodes.iter().copied().enumerate() {
        if seam_indices.contains(&index) {
            continue;
        }
        let center = curve_point_pixels(left, top, width, height, phase, node);
        let active = state.active_node() == Some(index);
        let selected = state.selected_node(index);
        let hovered = state.hover_node() == Some(index);
        let node_size = if active || selected {
            CURVE_NODE_SIZE + 1.7
        } else if hovered {
            CURVE_NODE_SIZE + 1.275
        } else {
            CURVE_NODE_SIZE
        };
        let node_bounds = Bounds::from_corners(
            point(center.x - px(node_size), center.y - px(node_size)),
            point(center.x + px(node_size), center.y + px(node_size)),
        );
        let fill_color = if active || selected {
            theme.accent_warning
        } else if hovered {
            theme.accent_mint
        } else {
            theme.surface_overlay
        };
        let stroke_color = if selected || (active && hovered) {
            theme.accent_mint
        } else if hovered {
            theme.accent_warning
        } else {
            theme.accent_copper
        };
        let stroke_width = if hovered { 1.275 } else { 1.0 };
        let endpoint = index == 0 || index + 1 == curve.nodes.len();
        if endpoint {
            let mut node_fill = gpui::PathBuilder::fill();
            for step in 0..=16 {
                let angle = std::f32::consts::TAU * step as f32 / 16.0;
                let node_point = point(
                    center.x + px(node_size * 0.7 * angle.cos()),
                    center.y + px(node_size * 0.7 * angle.sin()),
                );
                if step == 0 {
                    node_fill.move_to(node_point);
                } else {
                    node_fill.line_to(node_point);
                }
            }
            node_fill.close();
            if let Ok(path) = node_fill.build() {
                window.paint_path(path, solid(fill_color));
            }
        } else {
            window.paint_quad(fill(node_bounds, solid(fill_color)));
        }
        let mut node_path = gpui::PathBuilder::stroke(px(stroke_width));
        if endpoint {
            for step in 0..=16 {
                let angle = std::f32::consts::TAU * step as f32 / 16.0;
                let node_point = point(
                    center.x + px(node_size * 0.7 * angle.cos()),
                    center.y + px(node_size * 0.7 * angle.sin()),
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
            window.paint_path(path, solid(stroke_color));
        }
    }
    if let Some((start, current)) = state.active_curve_marquee() {
        let start = curve_point_pixels(left, top, width, height, phase, start);
        let current = curve_point_pixels(left, top, width, height, phase, current);
        let marquee_bounds = Bounds::from_corners(
            point(
                px(f32::from(start.x).min(f32::from(current.x))),
                px(f32::from(start.y).min(f32::from(current.y))),
            ),
            point(
                px(f32::from(start.x).max(f32::from(current.x))),
                px(f32::from(start.y).max(f32::from(current.y))),
            ),
        );
        window.paint_quad(fill(
            marquee_bounds,
            solid(theme.accent_mint.with_alpha(32)),
        ));
        let mut marquee = gpui::PathBuilder::stroke(px(1.0));
        marquee.move_to(point(marquee_bounds.left(), marquee_bounds.top()));
        marquee.line_to(point(marquee_bounds.right(), marquee_bounds.top()));
        marquee.line_to(point(marquee_bounds.right(), marquee_bounds.bottom()));
        marquee.line_to(point(marquee_bounds.left(), marquee_bounds.bottom()));
        marquee.close();
        if let Ok(path) = marquee.build() {
            window.paint_path(path, solid(theme.accent_mint));
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
        format!("{reduction:.1}"),
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
    let handle_width = PUMP_VISUAL_METRICS.space_16.min(width);
    let offset_x = left + (-phase).rem_euclid(1.0) * (width - handle_width).max(0.0);
    let handle_bounds = Bounds::from_corners(
        point(px(offset_x), px(offset_y)),
        point(
            px(offset_x + handle_width),
            px(offset_y + CURVE_OFFSET_BAR_HEIGHT),
        ),
    );
    let handle_hovered = last_pointer.is_some_and(|pointer| handle_bounds.contains(&pointer));
    window.paint_quad(fill(
        handle_bounds,
        solid(if state.active_curve_offset() {
            CURVE_OFFSET_MOVE_COLOR
        } else if handle_hovered {
            CURVE_OFFSET_HOVER_COLOR
        } else {
            theme.accent_mint
        }),
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
    let text = if target == NumericEntryTarget::FreeRate {
        state.format_free_rate(plain)
    } else {
        format_plain_value_text(id, plain as f64).unwrap_or_else(|| format!("{plain:.2}"))
    };
    (normalized, text)
}

fn button(
    id: &'static str,
    label: String,
    active: bool,
    width: f32,
    focus_handle: Option<&FocusHandle>,
) -> gpui::Stateful<gpui::Div> {
    let theme = pump_theme();
    let mut button = div()
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
        .line_height(px(PUMP_TYPOGRAPHY.body.1));
    if let Some(focus_handle) = focus_handle {
        let focus_handle_for_mouse = focus_handle.clone();
        button = button.track_focus(focus_handle).on_mouse_down(
            MouseButton::Left,
            move |_, window, cx| {
                window.focus(&focus_handle_for_mouse, cx);
            },
        );
    }
    button.child(label)
}

#[derive(Clone, Copy)]
enum IconKind {
    ChevronLeft,
    ChevronRight,
}

fn icon_button(
    id: &'static str,
    kind: IconKind,
    active: bool,
    width: f32,
    focus_handle: Option<&FocusHandle>,
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
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size_full();
    button(id, String::new(), active, width, focus_handle).child(icon)
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
            .on_scroll_wheel(cx.listener(move |view, event, window, cx| {
                view.knob_wheel(target, event, window, cx)
            }))
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
    button(id, String::new(), loaded || deviated, 1.0, None)
        .flex_1()
        .h(px(SLOT_HEIGHT))
        .rounded(px(PUMP_VISUAL_METRICS.radius))
        .child(preview)
}

impl Render for PumpEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        if self.focus_out_subscription.is_none() {
            let focus_handle = self.editor_focus_handle.clone();
            self.focus_out_subscription =
                Some(cx.on_focus_out(&focus_handle, window, |view, _, _, _| {
                    view.button_activation_keys.clear()
                }));
        }
        self.pump_tick(window, cx);
        let state = self.state.borrow();
        let theme = pump_theme();
        let params = state.params();
        let active_sound = params.active_sound();
        let bypassed = params.bypassed();
        let timing_free = params.timing_mode() == TIMING_MODE_FREE;
        let curve_bounds = Rc::clone(&self.curve_bounds);
        let draw_state = Rc::clone(&self.state);
        let editor_entity = cx.entity().downgrade();
        let last_pointer = self.last_pointer;
        let curve = canvas(
            move |bounds, _, _| {
                *curve_bounds.borrow_mut() = Some(bounds);
            },
            move |bounds, _, window, cx| {
                draw_curve(bounds, &draw_state.borrow(), last_pointer, window, cx);
                let down_editor_entity = editor_entity.clone();
                window.on_mouse_event(move |_: &MouseDownEvent, phase, _window, cx| {
                    if phase == DispatchPhase::Capture {
                        let consumed = down_editor_entity
                            .update(cx, |view, cx| view.captured_mouse_down(cx))
                            .unwrap_or(false);
                        if consumed {
                            cx.stop_propagation();
                        }
                    }
                });
                let move_editor_entity = editor_entity.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let consumed = move_editor_entity
                            .update(cx, |view, cx| view.captured_mouse_move(event, window, cx))
                            .unwrap_or(false);
                        if consumed {
                            cx.stop_propagation();
                        }
                    }
                });
                let up_editor_entity = editor_entity.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let consumed = up_editor_entity
                            .update(cx, |view, cx| view.captured_mouse_up(event, window, cx))
                            .unwrap_or(false);
                        if consumed {
                            cx.stop_propagation();
                        }
                    }
                });
                let exit_editor_entity = editor_entity.clone();
                window.on_mouse_event(move |_: &MouseExitEvent, phase, _window, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = exit_editor_entity.update(cx, |view, cx| {
                            view.captured_mouse_exit(cx);
                        });
                    }
                });
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
                let mut slot =
                    curve_slot_element(CURVE_SLOT_IDS[index], slot_curve, loaded, deviated);
                slot = slot.on_click(cx.listener(move |view, event, window, cx| {
                    view.slot_click(index, event, window, cx)
                }));
                slot
            }));
        let divider = |id: &'static str| {
            div()
                .id(id)
                .w(px(PUMP_VISUAL_METRICS.divider))
                .h(px(DECK_HEIGHT - 13.6))
                .bg(solid(theme.grid_strong))
        };
        let mut deck_children = vec![
            self.knob_element(NumericEntryTarget::Smooth, cx),
            divider("deck-divider-smooth"),
            self.knob_element(NumericEntryTarget::Swing, cx),
        ];
        if timing_free {
            deck_children.push(divider("deck-divider-free-rate"));
            deck_children.push(self.knob_element(NumericEntryTarget::FreeRate, cx));
        }
        deck_children.extend([
            divider("deck-divider-mix"),
            self.knob_element(NumericEntryTarget::Mix, cx),
            self.knob_element(NumericEntryTarget::OutputGain, cx),
        ]);
        let deck = div()
            .h(px(DECK_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .children(deck_children);
        let mut timing_button = button(
            "timing-mode",
            if timing_free {
                "FREE".into()
            } else {
                "SYNC".into()
            },
            timing_free,
            54.4,
            Some(self.button_focus_handle("timing-mode")),
        )
        .h(px(HEADER_CONTROL_HEIGHT));
        timing_button = timing_button.on_click(cx.listener(Self::toggle_timing));
        let timing_label = if timing_free {
            state.free_rate_unit().label().to_string()
        } else {
            format!("Sync {}", sync_division_label(params.sync_division()))
        };
        let timing_chevron = canvas(
            |_bounds, _, _| {},
            move |bounds, _, window, _cx| {
                let center_x = f32::from(bounds.left()) + f32::from(bounds.size.width) * 0.5;
                let center_y = f32::from(bounds.top()) + f32::from(bounds.size.height) * 0.5;
                let mut path = gpui::PathBuilder::stroke(px(1.15));
                path.move_to(point(px(center_x - 2.8), px(center_y - 1.2)));
                path.line_to(point(px(center_x), px(center_y + 1.6)));
                path.line_to(point(px(center_x + 2.8), px(center_y - 1.2)));
                if let Ok(path) = path.build() {
                    window.paint_path(path, solid(theme.text_muted));
                }
            },
        )
        .w(px(9.0))
        .h(px(9.0));
        let timing_menu = if state.timing_dropdown_open() {
            let options: Vec<_> = if timing_free {
                FreeRateUnit::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, unit)| {
                        let id = TIMING_FREE_RATE_IDS[index];
                        let mut option = button(
                            id,
                            unit.label().to_string(),
                            unit == state.free_rate_unit(),
                            95.2,
                            None,
                        )
                        .h(px(24.0));
                        option = option.on_click(cx.listener(move |view, event, window, cx| {
                            view.select_free_rate_unit(unit, event, window, cx)
                        }));
                        option
                    })
                    .collect()
            } else {
                SYNC_DIVISIONS
                    .iter()
                    .enumerate()
                    .map(|(index, division)| {
                        let id = TIMING_SYNC_IDS[index];
                        let mut option = button(
                            id,
                            division.label.to_string(),
                            index == params.sync_division(),
                            95.2,
                            None,
                        )
                        .h(px(24.0));
                        option = option.on_click(cx.listener(move |view, event, window, cx| {
                            view.select_sync_division(index, event, window, cx)
                        }));
                        option
                    })
                    .collect()
            };
            div()
                .id("timing-dropdown-menu")
                .absolute()
                .top(px(HEADER_CONTROL_HEIGHT + 2.0))
                .left(px(0.0))
                .w(px(95.2))
                .flex()
                .flex_col()
                .gap(px(1.0))
                .p(px(2.0))
                .bg(solid(theme.clear))
                .border_1()
                .border_color(solid(theme.border_emphasis))
                .children(options)
        } else {
            div().id("timing-dropdown-menu").hidden()
        };
        let mut timing_value = button(
            "timing-value",
            timing_label,
            false,
            95.2,
            Some(self.button_focus_handle("timing-value")),
        )
        .relative()
        .h(px(HEADER_CONTROL_HEIGHT))
        .child(timing_chevron)
        .child(gpui::deferred(timing_menu.occlude()));
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    let input = view.numeric_inputs
                        [Self::numeric_input_index(NumericEntryTarget::Delay)]
                    .clone();
                    input.update(cx, |input, cx| {
                        input.set_editing(true);
                        window.focus(&input.focus_handle, cx);
                    });
                    view.begin_numeric_input(NumericEntryTarget::Delay, cx);
                    cx.stop_propagation();
                }),
            )
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
        let mut undo_button = icon_button(
            "undo",
            IconKind::ChevronLeft,
            false,
            28.0,
            Some(self.button_focus_handle("undo")),
        )
        .h(px(HEADER_CONTROL_HEIGHT));
        undo_button = undo_button.on_click(cx.listener(Self::undo));
        let mut redo_button = icon_button(
            "redo",
            IconKind::ChevronRight,
            false,
            28.0,
            Some(self.button_focus_handle("redo")),
        )
        .h(px(HEADER_CONTROL_HEIGHT));
        redo_button = redo_button.on_click(cx.listener(Self::redo));
        let mut sound_a_button = button(
            "sound-a",
            "A".into(),
            false,
            28.0,
            Some(self.button_focus_handle("sound-a")),
        )
        .h(px(HEADER_CONTROL_HEIGHT));
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
            Some(self.button_focus_handle("sound-switch")),
        )
        .h(px(HEADER_CONTROL_HEIGHT));
        sound_switch = sound_switch.on_click(cx.listener(Self::select_sound_switch));
        let mut sound_b_button = button(
            "sound-b",
            "B".into(),
            false,
            28.0,
            Some(self.button_focus_handle("sound-b")),
        )
        .h(px(HEADER_CONTROL_HEIGHT));
        if active_sound == SoundSide::B {
            sound_b_button = sound_b_button
                .border_color(solid(theme.accent_copper))
                .bg(solid(theme.clear))
                .text_color(solid(theme.accent_copper));
        }
        sound_b_button = sound_b_button.on_click(cx.listener(Self::select_sound_b));
        let mut help_button = button(
            "hotkey-help",
            "?".into(),
            false,
            28.0,
            Some(self.button_focus_handle("hotkey-help")),
        )
        .h(px(HEADER_CONTROL_HEIGHT));
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
        let brand_meta = if params.preset_persistence_warning().is_some() {
            super::PRESET_WARNING_STORAGE.to_owned()
        } else {
            crate::gui::build_version_label()
        };
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
                    .child(brand_meta),
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
            Some(self.button_focus_handle("waveform-mode")),
        )
        .h(px(FOOTER_HEIGHT));
        waveform_button = waveform_button.on_click(cx.listener(Self::toggle_waveform));
        let mut bypass_button = button(
            "bypass",
            String::new(),
            bypassed,
            125.8,
            Some(self.button_focus_handle("bypass")),
        )
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
        let hotkey_help = if state.hotkey_help_open() {
            const ROWS: [(&str, &str); 10] = [
                ("u", "Undo"),
                ("U", "Redo"),
                ("Shift + drag node", "Lock gain"),
                ("Shift + Option + drag node", "Lock time"),
                ("Cmd + drag node", "Snap to beat grid"),
                ("Shift + drag canvas", "Marquee select nodes"),
                ("Option + drag segment", "Adjust segment tension"),
                ("Cmd + drag segment", "Move segment"),
                ("Cmd + Shift + drag canvas", "Offset the curve"),
                (
                    "Cmd + Shift + Option + drag canvas",
                    "Quantize curve offset",
                ),
            ];
            let rows = ROWS.into_iter().map(|(key, description)| {
                div()
                    .w_full()
                    .h(px(20.4))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .w(px(176.8))
                            .text_color(solid(theme.text_primary))
                            .font(font("Ioskeley Mono"))
                            .text_size(px(PUMP_TYPOGRAPHY.control_label.0))
                            .line_height(px(PUMP_TYPOGRAPHY.control_label.1))
                            .child(key),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_color(solid(theme.text_muted))
                            .font(font("Ioskeley Mono"))
                            .text_size(px(PUMP_TYPOGRAPHY.control_label.0))
                            .line_height(px(PUMP_TYPOGRAPHY.control_label.1))
                            .child(description),
                    )
            });
            div()
                .id("hotkey-help-overlay")
                .absolute()
                .top(px(HEADER_HEIGHT + SURFACE_SPACING))
                .right(px(SURFACE_PADDING))
                .w(px(306.0))
                .h(px(272.0))
                .p(px(13.6))
                .flex()
                .flex_col()
                .gap(px(0.0))
                .bg(solid(theme.surface_overlay))
                .border_1()
                .border_color(solid(theme.border_emphasis))
                .rounded(px(6.8))
                .child(
                    div()
                        .h(px(25.5))
                        .w_full()
                        .text_color(solid(theme.accent_copper))
                        .font(font("Ioskeley Mono"))
                        .text_size(px(PUMP_TYPOGRAPHY.body.0))
                        .line_height(px(PUMP_TYPOGRAPHY.body.1))
                        .child("PUMP HOTKEYS"),
                )
                .children(rows)
        } else {
            div().id("hotkey-help-overlay").hidden()
        };
        div()
            .id("pump-editor")
            .track_focus(&self.editor_focus_handle)
            .relative()
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
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_key_up(cx.listener(Self::handle_key_up))
            .on_modifiers_changed(cx.listener(Self::handle_modifiers))
            .child(header)
            .child(div().h(px(PUMP_VISUAL_METRICS.space_4)))
            .child(curve_area)
            .child(slots)
            .child(deck)
            .child(footer)
            .child(hotkey_help)
    }
}
