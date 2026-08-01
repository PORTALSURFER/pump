//! Shared Radiant surface for Pump editor hosts.

use std::sync::Arc;

use radiant::gui::automation::{AutomationLiveRegion, AutomationNodeSemantics, AutomationRole};
use radiant::gui::svg::IconName;
use radiant::gui::types::{Point, Rect, Rgba8, Vector2};
use radiant::gui::visualization::{
    push_sampled_curve_area_fill, SampledCurveAreaBaseline, SampledCurveAreaFillParts,
};
use radiant::layout::{CrossAlign, LayoutOutput, MainAlign};
use radiant::prelude::{
    column, custom_widget, custom_widget_direct, custom_widget_mapped, dismissible_overlay,
    dropdown_menu_overlay_below, dropdown_trigger, knob, row, spacer, stack, text, toggle,
    DropdownOption, IntoView, KnobMessage, TextAlign, TextColorRole, ViewNode,
};
#[cfg(test)]
use radiant::runtime::SurfaceFrame;
use radiant::runtime::{
    DeclarativeSurfaceRuntime, PaintBrush, PaintFillPath, PaintFillPolygon, PaintFillRect,
    PaintFillRule, PaintLinearGradient, PaintPath, PaintPathCommand, PaintPrimitive,
    PaintStrokePolyline, PaintStrokeRect, PaintText, PaintTextAlign, PaintTextRun, UiSurface,
};
#[cfg(any(feature = "radiant-gui", feature = "vst3"))]
use radiant::runtime::{Event, SurfacePaintPlan};
use radiant::theme::ThemeTokens;
use radiant::widgets::{
    ButtonMessage, ButtonWidget, FocusBehavior, IconButtonWidget, PointerButton, TextWrap, Widget,
    WidgetCapabilities, WidgetCommon, WidgetInput, WidgetKey, WidgetOutput, WidgetSemantics,
    WidgetSizing,
};
use toybox::clack_extensions::params::HostParams;
use toybox::clack_plugin::prelude::HostSharedHandle;
use toybox::clack_plugin::utils::ClapId;
use toybox::clap::automation::AutomationConfig;

use crate::automation_queue::PumpAutomationQueue;
use crate::curve::{
    sample_editable_curve, CurveNode, CurveSegment, EditableCurve, MAX_EDITABLE_NODES,
    MAX_SEGMENT_TENSION, MIN_SEGMENT_TENSION,
};
use crate::incoming_waveform::IncomingWaveformSnapshot;
use crate::params::{
    format_plain_value_text, normalized_from_plain_value, parse_plain_value_text,
    plain_from_normalized_value, sync_division_label, PumpParams, PumpSoundState, SoundSide,
    BYPASS_ACTIVE_VALUE, BYPASS_BYPASSED_VALUE, BYPASS_LABELS, DEFAULT_FREE_RATE_HZ, DEFAULT_MIX,
    DEFAULT_OUTPUT_GAIN_DB, DEFAULT_SMOOTH, GLOBAL_CURVE_SLOT_COUNT, MAX_OUTPUT_GAIN_DB,
    MAX_SYNC_DIVISION, MIN_OUTPUT_GAIN_DB, PARAM_BYPASS_ID, PARAM_FREE_RATE_ID, PARAM_MIX_ID,
    PARAM_OUTPUT_GAIN_ID, PARAM_PHASE_OFFSET_ID, PARAM_SMOOTH_ID, PARAM_SOUND_ID, PARAM_SWING_ID,
    PARAM_SYNC_DIVISION_ID, PARAM_TIMING_MODE_ID, SYNC_DIVISIONS, TIMING_MODE_FREE,
    TIMING_MODE_SYNC,
};
use crate::GuiStatus;

use super::curve_paint::{
    reconstruct_paint, PaintCommitOutcome, PaintRun, RectBounds, RectPoint, StrokeRecorder,
};
use super::visual_system::{pump_meter_colors, pump_theme, PUMP_TYPOGRAPHY, PUMP_VISUAL_METRICS};
#[cfg(test)]
use super::WINDOW_HEIGHT;
use super::{build_version_label, snap_curve_time_to_beat_grid_with_swing, WINDOW_WIDTH};

/// Main-thread CLAP parameter flush callback retained by the hosted editor.
#[derive(Clone, Copy)]
pub(crate) struct HostParamFlushRequester {
    host: HostSharedHandle<'static>,
    params: HostParams,
}

impl HostParamFlushRequester {
    /// Capture the host parameter extension when the host exposes it.
    pub(crate) fn new(host: HostSharedHandle<'_>) -> Option<Self> {
        let params = host.get_extension::<HostParams>()?;
        let host =
            unsafe { std::mem::transmute::<HostSharedHandle<'_>, HostSharedHandle<'static>>(host) };
        Some(Self { host, params })
    }

    /// Ask the host to collect the queued parameter events.
    fn request_flush(self) {
        self.params.request_flush(&self.host);
    }
}

/// Format-neutral sink for one complete UI-originated host parameter edit.
pub(crate) trait HostParamEditSink: Send + Sync {
    /// Deliver begin/value/end on the editor's host/UI thread.
    fn edit(&self, config: &AutomationConfig, param_id: ClapId, value: f64) -> bool;

    /// Begin a continuous knob gesture.
    fn gesture_started(&self, config: &AutomationConfig, param_id: ClapId) -> bool;

    /// Deliver one ordered value in a continuous knob gesture.
    fn gesture_value(&self, config: &AutomationConfig, param_id: ClapId, value: f64) -> bool;

    /// End a continuous knob gesture.
    fn gesture_ended(&self, config: &AutomationConfig, param_id: ClapId) -> bool;
}

struct ClapHostParamEditSink {
    queue: Arc<PumpAutomationQueue>,
    requester: Option<HostParamFlushRequester>,
}

impl HostParamEditSink for ClapHostParamEditSink {
    fn edit(&self, config: &AutomationConfig, param_id: ClapId, value: f64) -> bool {
        let complete = self.queue.push_gesture_edit(config, param_id, value);
        if complete {
            if let Some(requester) = self.requester {
                requester.request_flush();
            }
        }
        complete
    }

    fn gesture_started(&self, config: &AutomationConfig, param_id: ClapId) -> bool {
        let accepted = self.queue.push_gesture_begin(config, param_id);
        if accepted {
            if let Some(requester) = self.requester {
                requester.request_flush();
            }
        }
        accepted
    }

    fn gesture_value(&self, config: &AutomationConfig, param_id: ClapId, value: f64) -> bool {
        let accepted = self.queue.push_gesture_value(config, param_id, value);
        if accepted {
            if let Some(requester) = self.requester {
                requester.request_flush();
            }
        }
        accepted
    }

    fn gesture_ended(&self, config: &AutomationConfig, param_id: ClapId) -> bool {
        let complete = self.queue.push_gesture_end(config, param_id);
        if complete {
            if let Some(requester) = self.requester {
                requester.request_flush();
            }
        }
        complete
    }
}

const BUILD_LABEL_HEIGHT: f32 = 45.9;
const TIMING_CONTROL_HEIGHT: f32 = 34.0;
const HEADER_TO_CURVE_GAP: f32 = PUMP_VISUAL_METRICS.space_4;
const TIMING_MODE_TOGGLE_WIDTH: f32 = 54.4;
const TIMING_DROPDOWN_WIDTH: f32 = 95.2;
const HEADER_BRAND_WIDTH: f32 = 153.0;
// Leave room for the wider native fallback used by offscreen captures.
const HEADER_BRAND_WORDMARK_WIDTH: f32 = 98.0;
const HEADER_BRAND_TITLE_HEIGHT: f32 = 27.2;
const HEADER_BRAND_META_HEIGHT: f32 = 13.6;
const CURVE_PREVIEW_HEIGHT: f32 = 153.0;
const PARAMETER_DECK_HEIGHT: f32 = PUMP_VISUAL_METRICS.deck_height;
const GAIN_REDUCTION_METER_WIDTH: f32 = PUMP_VISUAL_METRICS.meter_panel;
const GAIN_REDUCTION_METER_BAR_WIDTH: f32 = PUMP_VISUAL_METRICS.meter_track;
// Compact swatches use 80% of the previous row height and gap while retaining
// equal fluid widths across the eight-slot row.
const CURVE_SLOT_ROW_HEIGHT: f32 = 40.8;
const CURVE_SLOT_SPACING: f32 = 2.72;
// Keep the dB/reference labels readable while giving the curve viewport back
// a modest amount of horizontal space.
const CURVE_REFERENCE_GUTTER_WIDTH: f32 = 40.8;
const CURVE_METER_GAP: f32 = 2.72;
const CONTROL_ROW_HEIGHT: f32 = PUMP_VISUAL_METRICS.label_line;
// Timing and parameter labels are intentionally wider than the compact legacy
// shell so supported values retain their full glyph bounds at the minimum host size.
const CONTROL_VALUE_WIDTH: f32 = 66.3;
const SURFACE_PADDING: f32 = PUMP_VISUAL_METRICS.padding;
const SURFACE_SPACING: f32 = PUMP_VISUAL_METRICS.divider;
const CURVE_SAMPLE_COUNT: usize = 96;
const CURVE_OFFSET_BAR_HEIGHT: f32 = 10.2;
const CURVE_OFFSET_BAR_INSET: f32 = 2.55;
const CURVE_OFFSET_HANDLE_WIDTH: f32 = 5.95;
const CURVE_FILL_TOP_ALPHA: u8 = 96;
const CURVE_FILL_BOTTOM_ALPHA: u8 = 12;
const CURVE_NODE_SIZE: f32 = 4.25;
const CURVE_PREVIEW_NODE_SIZE: f32 = 5.95;
const CURVE_NODE_HIT_RADIUS: f32 = 10.0;
const CURVE_NODE_INSERT_GUARD_RADIUS: f32 = 12.0;
const CURVE_SEGMENT_HOVER_RADIUS: f32 = 7.0;
const CURVE_SEGMENT_TENSION_PIXEL_SCALE: f32 = 120.0;
const CURVE_NODE_PUSH_THROUGH_MARGIN_PX: f32 = 10.0;
const CURVE_NODE_MIN_SPACING_X: f32 = 1.0e-3;
// Keep cyclic phase seams stable when a pointer is exactly on a viewport edge
// or the active offset point. The right edge must remain distinguishable from
// the left edge while a node is being dragged across a seam.
const CURVE_DISPLAY_SEAM_EPSILON: f32 = 1.0e-5;
const CURVE_PLAYHEAD_MARKER_HEIGHT: f32 = 4.25;
const CURVE_PLAYHEAD_MARKER_WIDTH: f32 = 7.65;
const CURVE_PLAYHEAD_CORE_COLOR: Rgba8 = Rgba8::new(128, 132, 132, 255);
const CURVE_SEGMENT_MOVE_COLOR: Rgba8 = Rgba8::new(96, 176, 255, 255);
const CURVE_OFFSET_MOVE_COLOR: Rgba8 = Rgba8::new(255, 168, 88, 255);
const CURVE_OFFSET_HOVER_COLOR: Rgba8 = CURVE_OFFSET_MOVE_COLOR.with_alpha(224);
const CURVE_PAINT_PREVIEW_WIDTH: f32 = 2.25;
const CURVE_REFERENCE_LABEL_HEIGHT: f32 = 10.2;
const CURVE_REFERENCE_FONT_SIZE: f32 = PUMP_TYPOGRAPHY.meta.0;
const CURVE_SLOT_PREVIEW_STEPS: usize = 24;
const CURVE_SLOT_MARGIN: f32 = 2.55;
const VALUE_ENTRY_MAX_CHARS: usize = 16;
const VALUE_LABEL_FONT_SIZE: f32 = PUMP_TYPOGRAPHY.value.0;
const BYPASS_CONTROL_WIDTH: f32 = 125.8;
const WAVEFORM_MODE_CONTROL_WIDTH: f32 = 61.2;
// Keep the compact header glyphs optically centered against the neighboring
// controls; the extra lower inset compensates for their font's ascender-heavy
// shape without changing the widget's hit rectangle.
const HEADER_BUTTON_TEXT_TOP_INSET: f32 = PUMP_VISUAL_METRICS.base;

fn header_button_hover_fill(theme: &ThemeTokens) -> Rgba8 {
    theme
        .surface_base
        .blend_toward(theme.surface_overlay, theme.state_hover_strong)
}

fn header_button_text_rect(bounds: Rect) -> Rect {
    Rect::from_xy_size(
        bounds.min.x,
        bounds.min.y + HEADER_BUTTON_TEXT_TOP_INSET,
        bounds.width(),
        (bounds.height() - HEADER_BUTTON_TEXT_TOP_INSET).max(1.0),
    )
}

fn header_button_text_baseline(rect: Rect, font_size: f32) -> f32 {
    (rect.height() * 0.5 + font_size * 0.35).max(0.0)
}

#[derive(Clone, Debug)]
struct BypassControlWidget {
    button: IconButtonWidget,
    bypassed: bool,
    pulse_phase: Option<f32>,
}

impl BypassControlWidget {
    fn new(bypassed: bool, automation_active: bool) -> Self {
        let mut button = IconButtonWidget::new(
            0,
            IconName::Power.icon(),
            WidgetSizing::fixed(Vector2::new(BYPASS_CONTROL_WIDTH, CONTROL_ROW_HEIGHT)),
        );
        button.common.state.selected = bypassed;
        button.common.state.active = !bypassed;
        button.common.state.automation_active = automation_active;
        Self {
            button,
            bypassed,
            pulse_phase: None,
        }
    }

    fn with_pulse_phase(mut self, phase: Option<f32>) -> Self {
        self.pulse_phase = phase.map(|phase| phase.rem_euclid(1.0));
        self
    }

    fn state_text(&self) -> &'static str {
        BYPASS_LABELS[usize::from(self.bypassed)]
    }
}

impl Widget for BypassControlWidget {
    fn common(&self) -> &WidgetCommon {
        self.button.common()
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        self.button.common_mut()
    }

    fn handle_input(&mut self, bounds: Rect, input: WidgetInput) -> Option<WidgetOutput> {
        self.button.handle_input(bounds, input)
    }

    fn accepts_pointer_move(&self) -> bool {
        true
    }

    fn synchronize_from_previous(&mut self, previous: &dyn Widget) {
        let Some(previous) = previous.as_any().downcast_ref::<Self>() else {
            return;
        };
        let selected = self.bypassed;
        let automation_active = self.button.common.state.automation_active;
        self.button.synchronize_from_previous(&previous.button);
        self.button.common.state.selected = selected;
        self.button.common.state.active = !selected;
        self.button.common.state.automation_active = automation_active;
        self.pulse_phase = previous.pulse_phase;
    }

    fn capabilities(&self) -> WidgetCapabilities<'_> {
        WidgetCapabilities::new().semantics(self)
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.button.common.id,
            rect: bounds,
            color: if self.bypassed {
                theme.surface_overlay
            } else if self.button.common.state.hovered {
                theme.surface_raised
            } else {
                theme.surface_base
            },
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.button.common.id,
            rect: bounds,
            color: if self.bypassed {
                self.pulse_phase
                    .map(|phase| {
                        theme
                            .accent_danger
                            .with_alpha(if phase < 0.5 { 180 } else { 255 })
                    })
                    .unwrap_or(theme.accent_danger)
            } else {
                theme.border_emphasis
            },
            width: if self.bypassed { 1.7 } else { 1.0 },
        }));
        let icon_size = bounds.height().min(PUMP_VISUAL_METRICS.icon);
        self.button.icon.append_paint(
            primitives,
            self.button.common.id,
            Rect::from_xy_size(
                bounds.min.x + 8.5,
                bounds.min.y + (bounds.height() - icon_size) * 0.5,
                icon_size,
                icon_size,
            ),
        );
        primitives.push(PaintPrimitive::Text(PaintTextRun {
            widget_id: self.button.common.id,
            text: PaintText::from_static(self.state_text()),
            rect: Rect::from_xy_size(
                bounds.min.x + 28.9,
                bounds.min.y,
                (bounds.width() - 35.7).max(1.0),
                bounds.height(),
            ),
            font_size: PUMP_TYPOGRAPHY.control_label.0,
            baseline: None,
            color: theme.text_primary,
            align: PaintTextAlign::Center,
            wrap: TextWrap::None,
        }));
        if self.bypassed {
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.button.common.id,
                points: [
                    Point::new(bounds.min.x + 2.55, bounds.min.y + 3.4),
                    Point::new(bounds.min.x + 2.55, bounds.max.y - 3.4),
                ]
                .into(),
                color: if let Some(phase) = self.pulse_phase {
                    theme
                        .accent_danger
                        .with_alpha(if phase < 0.5 { 180 } else { 255 })
                } else {
                    theme.accent_danger
                },
                width: 2.55,
            }));
        }
        if self.button.common.state.automation_active {
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.button.common.id,
                points: [
                    Point::new(bounds.max.x - 2.55, bounds.min.y + 3.4),
                    Point::new(bounds.max.x - 2.55, bounds.max.y - 3.4),
                ]
                .into(),
                color: theme.accent_copper,
                width: 1.7,
            }));
        }
    }
}

impl WidgetSemantics for BypassControlWidget {
    fn automation_role(&self) -> AutomationRole {
        AutomationRole::Button
    }

    fn automation_label(&self) -> Option<String> {
        Some("Bypass".to_owned())
    }

    fn automation_description(&self) -> Option<String> {
        Some("Crossfade complete Pump output to original dry unity".to_owned())
    }

    fn automation_value_text(&self) -> Option<String> {
        Some(self.state_text().to_owned())
    }

    fn automation_checked(&self) -> Option<bool> {
        Some(self.bypassed)
    }
}

#[derive(Clone, Debug)]
struct ActionIconButtonWidget {
    button: IconButtonWidget,
    label: &'static str,
    disabled: bool,
}

#[derive(Clone, Debug)]
struct SoundSwitchButtonWidget {
    button: IconButtonWidget,
    active_sound: SoundSide,
    command_held: bool,
}

#[derive(Clone, Debug)]
struct SoundSideButtonWidget {
    button: ButtonWidget,
    side: SoundSide,
    selected: bool,
    alt_held: bool,
}

/// Compact question-mark control that opens the editor's hotkey reference.
#[derive(Clone, Debug)]
struct HotkeyHelpButtonWidget {
    button: ButtonWidget,
}

impl HotkeyHelpButtonWidget {
    fn new() -> Self {
        Self {
            button: ButtonWidget::new(
                0,
                "?",
                WidgetSizing::fixed(Vector2::new(
                    PUMP_VISUAL_METRICS.icon_hit,
                    TIMING_CONTROL_HEIGHT,
                )),
            )
            .with_hover_chrome_only(),
        }
    }
}

impl Widget for HotkeyHelpButtonWidget {
    fn common(&self) -> &WidgetCommon {
        self.button.common()
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        self.button.common_mut()
    }

    fn handle_input(&mut self, bounds: Rect, input: WidgetInput) -> Option<WidgetOutput> {
        self.button
            .handle_input(bounds, input)
            .map(WidgetOutput::typed)
    }

    fn accepts_pointer_move(&self) -> bool {
        true
    }

    fn synchronize_from_previous(&mut self, previous: &dyn Widget) {
        let Some(previous) = previous.as_any().downcast_ref::<Self>() else {
            return;
        };
        self.button.synchronize_from_previous(&previous.button);
    }

    fn capabilities(&self) -> WidgetCapabilities<'_> {
        WidgetCapabilities::new().semantics(self)
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        let text_rect = header_button_text_rect(bounds);
        let fill = if self.button.common.state.pressed {
            theme.accent_copper.with_alpha(96)
        } else if self.button.common.state.hovered {
            header_button_hover_fill(theme)
        } else {
            theme.surface_base
        };
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.button.common.id,
            rect: bounds,
            color: fill,
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.button.common.id,
            rect: bounds,
            color: if self.button.common.state.focused {
                theme.accent_warning
            } else {
                theme.border_emphasis
            },
            width: 1.0,
        }));
        primitives.push(PaintPrimitive::Text(PaintTextRun {
            widget_id: self.button.common.id,
            text: PaintText::from_static("?"),
            rect: text_rect,
            font_size: PUMP_TYPOGRAPHY.body.0,
            baseline: Some(header_button_text_baseline(
                text_rect,
                PUMP_TYPOGRAPHY.body.0,
            )),
            color: theme.text_primary,
            align: PaintTextAlign::Center,
            wrap: TextWrap::None,
        }));
    }
}

impl WidgetSemantics for HotkeyHelpButtonWidget {
    fn automation_role(&self) -> AutomationRole {
        AutomationRole::Button
    }

    fn automation_label(&self) -> Option<String> {
        Some("Show hotkeys".to_owned())
    }

    fn automation_description(&self) -> Option<String> {
        Some("Open the Pump hotkey reference".to_owned())
    }
}

impl SoundSwitchButtonWidget {
    fn new(active_sound: SoundSide) -> Self {
        Self {
            button: IconButtonWidget::new(
                0,
                if active_sound == SoundSide::A {
                    IconName::ChevronRight.icon()
                } else {
                    IconName::ChevronLeft.icon()
                },
                WidgetSizing::fixed(Vector2::new(
                    PUMP_VISUAL_METRICS.icon_hit,
                    TIMING_CONTROL_HEIGHT,
                )),
            ),
            active_sound,
            command_held: false,
        }
    }
}

impl Widget for SoundSwitchButtonWidget {
    fn common(&self) -> &WidgetCommon {
        self.button.common()
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        self.button.common_mut()
    }

    fn handle_input(&mut self, bounds: Rect, input: WidgetInput) -> Option<WidgetOutput> {
        match input {
            WidgetInput::PointerModifiersChanged { modifiers } => {
                self.command_held = modifiers.command;
                None
            }
            WidgetInput::PointerPress { modifiers, .. } => {
                self.command_held = modifiers.command;
                let _ = Widget::handle_input(&mut self.button, bounds, input);
                None
            }
            WidgetInput::PointerRelease { modifiers, .. } => {
                let command_held = self.command_held || modifiers.command;
                let output = Widget::handle_input(&mut self.button, bounds, input)
                    .and_then(|output| output.typed_copied::<ButtonMessage>())
                    .and_then(|message| {
                        message.is_activate().then(|| {
                            if command_held {
                                WidgetOutput::typed(RadiantEditorMessage::CopyAndSelectSound(
                                    self.active_sound.other(),
                                ))
                            } else {
                                WidgetOutput::typed(RadiantEditorMessage::SelectSound {
                                    side: self.active_sound.other(),
                                    copy: false,
                                })
                            }
                        })
                    });
                self.command_held = false;
                output
            }
            WidgetInput::KeyPress(WidgetKey::Enter | WidgetKey::Space) => self
                .button
                .handle_input(bounds, input)
                .and_then(|output| output.typed_copied::<ButtonMessage>())
                .and_then(|message| {
                    message.is_activate().then(|| {
                        WidgetOutput::typed(RadiantEditorMessage::SelectSound {
                            side: self.active_sound.other(),
                            copy: false,
                        })
                    })
                }),
            WidgetInput::PointerDrop { .. } => {
                let _ = Widget::handle_input(&mut self.button, bounds, input);
                self.command_held = false;
                None
            }
            _ => {
                let _ = Widget::handle_input(&mut self.button, bounds, input);
                None
            }
        }
    }

    fn synchronize_from_previous(&mut self, previous: &dyn Widget) {
        let Some(previous) = previous.as_any().downcast_ref::<Self>() else {
            return;
        };
        self.button.synchronize_from_previous(&previous.button);
    }

    fn capabilities(&self) -> WidgetCapabilities<'_> {
        WidgetCapabilities::new().semantics(self)
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        let state = &self.button.common.state;
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.button.common.id,
            rect: bounds,
            color: if state.pressed {
                theme.accent_copper.with_alpha(96)
            } else if state.hovered || state.focused {
                theme.surface_raised
            } else {
                theme.surface_base
            },
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.button.common.id,
            rect: bounds,
            color: if state.pressed {
                theme.accent_copper
            } else if state.hovered || state.focused {
                theme.border_emphasis
            } else {
                theme.border
            },
            width: 1.0,
        }));
        let icon_size = bounds.height().min(PUMP_VISUAL_METRICS.icon);
        self.button.icon.append_paint(
            primitives,
            self.button.common.id,
            Rect::from_xy_size(
                bounds.min.x + (bounds.width() - icon_size) * 0.5,
                bounds.min.y + (bounds.height() - icon_size) * 0.5,
                icon_size,
                icon_size,
            ),
        );
    }
}

impl WidgetSemantics for SoundSwitchButtonWidget {
    fn automation_role(&self) -> AutomationRole {
        AutomationRole::Button
    }

    fn automation_label(&self) -> Option<String> {
        Some("Switch sound".to_owned())
    }
}

impl SoundSideButtonWidget {
    fn new(side: SoundSide, selected: bool) -> Self {
        Self {
            button: ButtonWidget::new(
                0,
                side.label(),
                WidgetSizing::fixed(Vector2::new(28.9, TIMING_CONTROL_HEIGHT)),
            ),
            side,
            selected,
            alt_held: false,
        }
    }
}

impl Widget for SoundSideButtonWidget {
    fn common(&self) -> &WidgetCommon {
        self.button.common()
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        self.button.common_mut()
    }

    fn handle_input(&mut self, bounds: Rect, input: WidgetInput) -> Option<WidgetOutput> {
        match input {
            WidgetInput::PointerModifiersChanged { modifiers } => {
                self.alt_held = modifiers.alt;
                None
            }
            WidgetInput::PointerPress { modifiers, .. } => {
                self.alt_held = modifiers.alt;
                let _ = self.button.handle_input(bounds, input);
                None
            }
            WidgetInput::PointerRelease { modifiers, .. } => {
                let alt_held = self.alt_held || modifiers.alt;
                let activated = self
                    .button
                    .handle_input(bounds, input)
                    .is_some_and(ButtonMessage::is_activate);
                self.alt_held = false;
                if !activated {
                    return None;
                }
                Some(WidgetOutput::typed(RadiantEditorMessage::SelectSound {
                    side: self.side,
                    copy: alt_held && !self.selected,
                }))
            }
            WidgetInput::KeyPress(WidgetKey::Enter | WidgetKey::Space) => {
                self.button.handle_input(bounds, input).and_then(|message| {
                    message.is_activate().then(|| {
                        WidgetOutput::typed(RadiantEditorMessage::SelectSound {
                            side: self.side,
                            copy: false,
                        })
                    })
                })
            }
            WidgetInput::PointerDrop { .. } => {
                let _ = self.button.handle_input(bounds, input);
                self.alt_held = false;
                None
            }
            _ => {
                let _ = self.button.handle_input(bounds, input);
                None
            }
        }
    }

    fn accepts_pointer_move(&self) -> bool {
        true
    }

    fn synchronize_from_previous(&mut self, previous: &dyn Widget) {
        let Some(previous) = previous.as_any().downcast_ref::<Self>() else {
            return;
        };
        self.button.synchronize_from_previous(&previous.button);
        self.alt_held = previous.alt_held;
    }

    fn capabilities(&self) -> WidgetCapabilities<'_> {
        WidgetCapabilities::new().semantics(self)
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        let text_rect = header_button_text_rect(bounds);
        let fill = if self.button.common.state.pressed {
            theme.accent_copper.with_alpha(96)
        } else if self.selected {
            theme.surface_raised.with_alpha(224)
        } else if self.button.common.state.hovered {
            header_button_hover_fill(theme)
        } else {
            theme.surface_base
        };
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.button.common.id,
            rect: bounds,
            color: fill,
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.button.common.id,
            rect: bounds,
            color: if self.button.common.state.pressed {
                theme.accent_copper
            } else if self.selected || self.button.common.state.focused {
                theme.text_muted
            } else {
                theme.border_emphasis
            },
            width: 1.0,
        }));
        primitives.push(PaintPrimitive::Text(PaintTextRun {
            widget_id: self.button.common.id,
            text: PaintText::from_static(self.side.label()),
            rect: text_rect,
            font_size: PUMP_TYPOGRAPHY.body.0,
            baseline: Some(header_button_text_baseline(
                text_rect,
                PUMP_TYPOGRAPHY.body.0,
            )),
            color: theme.text_primary,
            align: PaintTextAlign::Center,
            wrap: TextWrap::None,
        }));
    }
}

impl WidgetSemantics for SoundSideButtonWidget {
    fn automation_role(&self) -> AutomationRole {
        AutomationRole::Button
    }

    fn automation_label(&self) -> Option<String> {
        Some(format!("Sound {}", self.side.label()))
    }

    fn automation_checked(&self) -> Option<bool> {
        Some(self.selected)
    }
}

#[derive(Clone, Debug)]
struct MetadataTextWidget {
    common: WidgetCommon,
    text: PaintText,
}

const HOTKEY_HELP_WIDTH: f32 = 306.0;
const HOTKEY_HELP_HEIGHT: f32 = 272.0;

/// Compact, non-interactive key reference shown over the editor surface.
#[derive(Clone)]
struct HotkeyHelpWidget {
    common: WidgetCommon,
}

impl HotkeyHelpWidget {
    fn new() -> Self {
        Self {
            common: WidgetCommon::fixed(0, HOTKEY_HELP_WIDTH, HOTKEY_HELP_HEIGHT)
                .without_default_chrome(),
        }
    }
}

impl Widget for HotkeyHelpWidget {
    fn common(&self) -> &WidgetCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        &mut self.common
    }

    fn handle_input(&mut self, _bounds: Rect, _input: WidgetInput) -> Option<WidgetOutput> {
        None
    }

    fn capabilities(&self) -> WidgetCapabilities<'_> {
        WidgetCapabilities::new().semantics(self)
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        let panel = bounds.inset(1.0, 1.0, 1.0, 1.0);
        primitives.push(PaintPrimitive::FillPath(PaintFillPath::new(
            self.common.id,
            PaintPath::from(rounded_rect_commands(panel, 6.8)),
            PaintBrush::solid(theme.surface_overlay),
        )));
        primitives.push(PaintPrimitive::FillPath(
            PaintFillPath::new(
                self.common.id,
                rounded_ring_path(bounds.inset(0.75, 0.75, 0.75, 0.75), 6.8, 1.0),
                PaintBrush::solid(theme.border_emphasis),
            )
            .fill_rule(PaintFillRule::EvenOdd),
        ));

        primitives.push(PaintPrimitive::Text(PaintTextRun {
            widget_id: self.common.id,
            text: PaintText::from_static("PUMP HOTKEYS"),
            rect: Rect::from_xy_size(
                bounds.min.x + 13.6,
                bounds.min.y + 10.2,
                bounds.width() - 27.2,
                17.0,
            ),
            font_size: PUMP_TYPOGRAPHY.body.0,
            baseline: None,
            color: theme.accent_copper,
            align: PaintTextAlign::Left,
            wrap: TextWrap::None,
        }));

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
        let key_width = 176.8;
        let row_top = bounds.min.y + 35.7;
        for (index, (key, description)) in ROWS.into_iter().enumerate() {
            let y = row_top + index as f32 * 20.4;
            primitives.push(PaintPrimitive::Text(PaintTextRun {
                widget_id: self.common.id,
                text: PaintText::from_static(key),
                rect: Rect::from_xy_size(bounds.min.x + 13.6, y, key_width, 17.0),
                font_size: PUMP_TYPOGRAPHY.control_label.0,
                baseline: None,
                color: theme.text_primary,
                align: PaintTextAlign::Left,
                wrap: TextWrap::None,
            }));
            primitives.push(PaintPrimitive::Text(PaintTextRun {
                widget_id: self.common.id,
                text: PaintText::from_static(description),
                rect: Rect::from_xy_size(
                    bounds.min.x + 13.6 + key_width,
                    y,
                    bounds.width() - key_width - 27.2,
                    17.0,
                ),
                font_size: PUMP_TYPOGRAPHY.control_label.0,
                baseline: None,
                color: theme.text_muted,
                align: PaintTextAlign::Left,
                wrap: TextWrap::None,
            }));
        }
    }
}

impl WidgetSemantics for HotkeyHelpWidget {
    fn automation_role(&self) -> AutomationRole {
        AutomationRole::Text
    }

    fn automation_label(&self) -> Option<String> {
        Some("Pump hotkeys".to_owned())
    }
}

impl MetadataTextWidget {
    fn new(text: impl Into<PaintText>) -> Self {
        Self {
            common: WidgetCommon::fixed(0, 1.0, HEADER_BRAND_META_HEIGHT),
            text: text.into(),
        }
    }
}

impl Widget for MetadataTextWidget {
    fn common(&self) -> &WidgetCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        &mut self.common
    }

    fn handle_input(&mut self, _bounds: Rect, _input: WidgetInput) -> Option<WidgetOutput> {
        None
    }

    fn capabilities(&self) -> WidgetCapabilities<'_> {
        WidgetCapabilities::new().semantics(self)
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        primitives.push(PaintPrimitive::Text(PaintTextRun {
            widget_id: self.common.id,
            text: self.text.clone(),
            rect: bounds,
            font_size: PUMP_TYPOGRAPHY.meta.0,
            baseline: None,
            color: theme.text_muted,
            align: PaintTextAlign::Right,
            wrap: TextWrap::None,
        }));
    }
}

impl WidgetSemantics for MetadataTextWidget {
    fn automation_role(&self) -> AutomationRole {
        AutomationRole::Text
    }

    fn automation_label(&self) -> Option<String> {
        Some(self.text.as_str().to_owned())
    }
}

impl ActionIconButtonWidget {
    fn new_with_state(
        icon: IconName,
        label: &'static str,
        width: f32,
        height: f32,
        disabled: bool,
    ) -> Self {
        let button = IconButtonWidget::new(
            0,
            icon.icon(),
            WidgetSizing::fixed(Vector2::new(width, height)),
        );
        Self {
            button,
            label,
            disabled,
        }
    }
}

impl Widget for ActionIconButtonWidget {
    fn common(&self) -> &WidgetCommon {
        self.button.common()
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        self.button.common_mut()
    }

    fn handle_input(&mut self, bounds: Rect, input: WidgetInput) -> Option<WidgetOutput> {
        if self.disabled {
            if let WidgetInput::PointerMove { position } = input {
                // Disabled actions remain hoverable so their explanatory
                // tooltip can be discovered without making them activatable.
                self.button.common.state.hovered = bounds.contains(position);
            }
            return None;
        }
        self.button.handle_input(bounds, input)
    }

    fn accepts_pointer_move(&self) -> bool {
        self.button.accepts_pointer_move()
    }

    fn synchronize_from_previous(&mut self, previous: &dyn Widget) {
        let Some(previous) = previous.as_any().downcast_ref::<Self>() else {
            return;
        };
        self.button.synchronize_from_previous(&previous.button);
    }

    fn capabilities(&self) -> WidgetCapabilities<'_> {
        WidgetCapabilities::new().semantics(self)
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        let state = &self.button.common.state;
        let fill = if self.disabled {
            theme.control_disabled_fill
        } else if state.pressed {
            theme.surface_raised.with_alpha(224)
        } else if state.hovered || state.focused {
            theme.surface_raised
        } else {
            theme.surface_base
        };
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.button.common.id,
            rect: bounds,
            color: fill,
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.button.common.id,
            rect: bounds,
            color: if state.focused {
                theme.text_muted
            } else if !self.disabled && (state.hovered || state.pressed) {
                theme.border_emphasis
            } else {
                theme.border
            },
            width: 1.0,
        }));
        let icon_size = bounds.height().min(PUMP_VISUAL_METRICS.icon);
        self.button.icon.append_paint(
            primitives,
            self.button.common.id,
            Rect::from_xy_size(
                bounds.min.x + (bounds.width() - icon_size) * 0.5,
                bounds.min.y + (bounds.height() - icon_size) * 0.5,
                icon_size,
                icon_size,
            ),
        );
    }
}

impl WidgetSemantics for ActionIconButtonWidget {
    fn automation_role(&self) -> AutomationRole {
        AutomationRole::Button
    }

    fn automation_label(&self) -> Option<String> {
        Some(self.label.to_owned())
    }

    fn resolve_automation_semantics(&self, common: &WidgetCommon) -> AutomationNodeSemantics {
        AutomationNodeSemantics {
            role: self.automation_role(),
            label: self.automation_label(),
            description: self.automation_description(),
            value_text: self.automation_value_text(),
            checked: self.automation_checked(),
            selected: common.state.selected,
            disabled: self.disabled,
            read_only: common.state.read_only,
            focusable: common.focus != FocusBehavior::None && !self.disabled,
            focused: common.state.focused,
            tab_index: (common.focus == FocusBehavior::Keyboard && !self.disabled).then_some(0),
            focus_hints: Default::default(),
            live_region: AutomationLiveRegion::None,
            metadata: Default::default(),
        }
    }
}

fn action_icon_button_with_state(
    icon: IconName,
    label: &'static str,
    message: RadiantEditorMessage,
    width: f32,
    height: f32,
    disabled: bool,
) -> ViewNode<RadiantEditorMessage> {
    custom_widget_mapped(
        ActionIconButtonWidget::new_with_state(icon, label, width, height, disabled),
        move |_: ButtonMessage| message.clone(),
    )
    .size(width, height)
}

fn curve_reference_gutter_width(preview_width: f32) -> f32 {
    CURVE_REFERENCE_GUTTER_WIDTH.min((preview_width - 1.0).max(0.0))
}

fn curve_viewport_width(preview_width: f32) -> f32 {
    (preview_width - curve_reference_gutter_width(preview_width)).max(1.0)
}

fn curve_node_push_through_threshold_x(preview_width: f32) -> f32 {
    CURVE_NODE_PUSH_THROUGH_MARGIN_PX
        / (curve_viewport_width(preview_width).max(1.0) - 1.0).max(1.0)
}

fn curve_width_from_push_through_threshold_x(threshold_x: f32) -> f32 {
    if threshold_x > f32::EPSILON {
        CURVE_NODE_PUSH_THROUGH_MARGIN_PX / threshold_x + 1.0
    } else {
        1.0
    }
}

fn interactive_curve_node_survivors(
    curve: &EditableCurve,
    phase_offset: f32,
    active_node: Option<usize>,
) -> Vec<usize> {
    if curve.nodes.is_empty() {
        return Vec::new();
    }

    let phase_offset = if phase_offset.is_finite() {
        phase_offset.rem_euclid(1.0)
    } else {
        0.0
    };
    let display_x: Vec<f32> = curve
        .nodes
        .iter()
        .map(|node| CurvePreviewWidget::display_phase(node.x, phase_offset))
        .collect();
    let left_survivor = select_interactive_edge_survivor(&display_x, active_node, true);
    let right_survivor = select_interactive_edge_survivor(&display_x, active_node, false);

    (0..curve.nodes.len())
        .filter(|index| {
            let x = display_x[*index];
            let in_left_band = x <= CURVE_NODE_MIN_SPACING_X;
            let in_right_band = x >= 1.0 - CURVE_NODE_MIN_SPACING_X;
            (!in_left_band && !in_right_band)
                || Some(*index) == left_survivor
                || Some(*index) == right_survivor
        })
        .collect()
}

fn select_interactive_edge_survivor(
    display_x: &[f32],
    active_node: Option<usize>,
    left_side: bool,
) -> Option<usize> {
    let last_index = display_x.len().saturating_sub(1);
    let in_band = |x: f32| {
        if left_side {
            x <= CURVE_NODE_MIN_SPACING_X
        } else {
            x >= 1.0 - CURVE_NODE_MIN_SPACING_X
        }
    };

    if let Some(active) =
        active_node.filter(|index| display_x.get(*index).copied().is_some_and(&in_band))
    {
        return Some(active);
    }

    let structural = if left_side { 0 } else { last_index };
    if display_x.get(structural).copied().is_some_and(|x| {
        in_band(x)
            && if left_side {
                x <= CURVE_DISPLAY_SEAM_EPSILON
            } else {
                x >= 1.0 - CURVE_DISPLAY_SEAM_EPSILON
            }
    }) {
        return Some(structural);
    }

    display_x
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, x)| in_band(*x))
        .min_by(|(left_index, left_x), (right_index, right_x)| {
            let left_distance = if left_side { *left_x } else { 1.0 - *left_x };
            let right_distance = if left_side { *right_x } else { 1.0 - *right_x };
            left_distance
                .total_cmp(&right_distance)
                .then_with(|| left_index.cmp(right_index))
        })
        .map(|(index, _)| index)
}

fn feasible_delta_interval_including_zero(min_delta: f32, max_delta: f32) -> (f32, f32) {
    if min_delta <= max_delta && min_delta <= 0.0 && max_delta >= 0.0 {
        (min_delta, max_delta)
    } else {
        // Legacy topology can make the preferred spacing interval empty or
        // exclude the current group position. Keep zero movement available so
        // a vertical-only edit remains a valid gesture.
        (min_delta.min(0.0), max_delta.max(0.0))
    }
}

fn feasible_node_x_interval_including_origin(min_x: f32, max_x: f32, origin_x: f32) -> (f32, f32) {
    if min_x <= max_x && min_x <= origin_x && origin_x <= max_x {
        (min_x, max_x)
    } else {
        // Preserve the legacy node's current x when the newer interactive
        // spacing policy cannot contain it. This also guarantees clamp bounds
        // stay ordered when the preferred interval is contradictory.
        (min_x.min(origin_x), max_x.max(origin_x))
    }
}

fn resolve_curve_offset(
    sync_division: usize,
    width: f32,
    swing: f32,
    origin: f32,
    delta: f32,
    snap_to_grid: bool,
) -> f32 {
    let phase = (origin + delta).rem_euclid(1.0);
    if snap_to_grid {
        snap_curve_time_to_beat_grid_with_swing(sync_division, width, phase, swing)
    } else {
        phase
    }
}

#[derive(Clone)]
struct ActiveCurveNodeDrag {
    origin_index: usize,
    origin_curve: EditableCurve,
    /// Marquee selection retained for a grouped node drag. An empty list is
    /// the ordinary single-node gesture.
    selected_indices: Vec<usize>,
    horizontal_gain_anchor: Option<f32>,
    vertical_time_anchor: Option<f32>,
    last_pointer_x: f32,
    last_pointer_y: f32,
    unconstrained_x_offset: f32,
    unconstrained_y_offset: f32,
    suppress_command_snap_once: bool,
}

#[derive(Clone, Copy)]
struct CurvePointDragModifiers {
    shift_held: bool,
    option_held: bool,
    command_held: bool,
}

impl ActiveCurveNodeDrag {
    fn set_constraints(&mut self, shift_held: bool, option_held: bool, current: CurveNode) {
        let vertical_active = shift_held && option_held;
        let horizontal_active = shift_held && !vertical_active;
        match (self.vertical_time_anchor, vertical_active) {
            (None, true) => {
                self.vertical_time_anchor = Some(current.x.clamp(0.0, 1.0));
            }
            (Some(anchor), false) => {
                self.unconstrained_x_offset = anchor - self.last_pointer_x;
                self.vertical_time_anchor = None;
                self.suppress_command_snap_once = true;
            }
            _ => {}
        }
        match (self.horizontal_gain_anchor, horizontal_active) {
            (None, true) => {
                self.horizontal_gain_anchor = Some(current.y.clamp(0.0, 1.0));
            }
            (Some(anchor), false) => {
                self.unconstrained_y_offset = anchor - self.last_pointer_y;
                self.horizontal_gain_anchor = None;
            }
            _ => {}
        }
    }

    fn target_for_pointer(
        &mut self,
        target: CurveNode,
        modifiers: CurvePointDragModifiers,
        sync_division: usize,
        curve_width: f32,
        swing: f32,
        current: CurveNode,
    ) -> CurveNode {
        self.last_pointer_x = target.x;
        self.last_pointer_y = target.y;
        self.set_constraints(modifiers.shift_held, modifiers.option_held, current);
        let suppress_command_snap = self.suppress_command_snap_once;
        self.suppress_command_snap_once = false;
        let mut effective = CurveNode {
            x: self
                .vertical_time_anchor
                .unwrap_or(target.x + self.unconstrained_x_offset)
                .clamp(0.0, 1.0),
            y: self
                .horizontal_gain_anchor
                .unwrap_or(target.y + self.unconstrained_y_offset)
                .clamp(0.0, 1.0),
        };
        if modifiers.command_held && self.vertical_time_anchor.is_none() && !suppress_command_snap {
            effective.x = snap_curve_time_to_beat_grid_with_swing(
                sync_division,
                curve_width,
                effective.x,
                swing,
            );
        }
        effective
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CurvePaintSample {
    node: CurveNode,
    display_position: RectPoint,
    outside: bool,
}

impl CurvePaintSample {
    fn raw_position(self) -> RectPoint {
        self.display_position
    }
}

#[derive(Clone)]
struct ActiveCurvePaint {
    origin_snapshot: RadiantHistorySnapshot,
    origin_curve: EditableCurve,
    phase_offset: f32,
    recorder: StrokeRecorder,
}

impl ActiveCurvePaint {
    fn new(origin_snapshot: RadiantHistorySnapshot, phase_offset: f32) -> Self {
        let origin_curve = origin_snapshot.curve.clone().normalized();
        Self {
            origin_snapshot,
            origin_curve,
            phase_offset,
            recorder: StrokeRecorder::new(RectBounds {
                min: RectPoint { x: 0.0, y: 0.0 },
                max: RectPoint { x: 1.0, y: 1.0 },
            }),
        }
    }

    fn push_sample(&mut self, sample: CurvePaintSample) {
        if sample.outside {
            self.recorder.observe_outside(sample.raw_position());
        } else {
            self.recorder.observe(sample.raw_position());
        }
    }

    fn push_boundary_sample(&mut self, sample: CurvePaintSample) {
        self.recorder.observe_outside(sample.raw_position());
    }

    fn preview_runs(&self) -> Vec<PaintRun> {
        self.recorder.runs().to_vec()
    }

    fn finished_curve(&self) -> PaintCommitOutcome {
        reconstruct_paint(&self.origin_curve, self.phase_offset, self.recorder.runs())
    }

    #[cfg(test)]
    fn preview_candidate(&self) -> EditableCurve {
        self.finished_curve().candidate().clone()
    }
}

#[derive(Clone)]
struct ActiveCurveSegmentDrag {
    index: usize,
    origin_curve: EditableCurve,
    start_pointer: Point,
    mode: CurveSegmentDragMode,
}

#[derive(Clone)]
struct ActiveCurveOffsetDrag {
    origin_phase_offset: f32,
    start_pointer_x: f32,
    raw_delta: f32,
    quantized: bool,
}

#[derive(Clone, Copy)]
struct ActiveCurveMarquee {
    start: CurveNode,
    current: CurveNode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CurveSegmentDragMode {
    AdjustTension { start_tension: f32 },
    MovePair,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumericEntryTarget {
    Mix,
    OutputGain,
    Smooth,
    Swing,
    FreeRate,
}

impl NumericEntryTarget {
    fn param_id(self) -> ClapId {
        match self {
            Self::Mix => PARAM_MIX_ID,
            Self::OutputGain => PARAM_OUTPUT_GAIN_ID,
            Self::Smooth => PARAM_SMOOTH_ID,
            Self::Swing => PARAM_SWING_ID,
            Self::FreeRate => PARAM_FREE_RATE_ID,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Mix => "Mix",
            Self::OutputGain => "Output",
            Self::Smooth => "Smooth",
            Self::Swing => "Swing",
            Self::FreeRate => "Free Rate",
        }
    }

    fn widget_key(self) -> &'static str {
        match self {
            Self::Mix => "numeric-entry-mix",
            Self::OutputGain => "numeric-entry-output",
            Self::Smooth => "numeric-entry-smooth",
            Self::Swing => "numeric-entry-swing",
            Self::FreeRate => "numeric-entry-free-rate",
        }
    }

    fn current_plain_value(self, params: &PumpParams) -> f64 {
        match self {
            Self::Mix => params.mix() as f64,
            Self::OutputGain => params.output_gain_db() as f64,
            Self::Smooth => params.smooth() as f64,
            Self::Swing => params.swing() as f64,
            Self::FreeRate => params.free_rate_hz() as f64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FreeRateUnit {
    Milliseconds,
    Seconds,
    Hertz,
    Kilohertz,
}

impl FreeRateUnit {
    const ALL: [Self; 4] = [
        Self::Milliseconds,
        Self::Seconds,
        Self::Hertz,
        Self::Kilohertz,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Milliseconds => "ms",
            Self::Seconds => "s",
            Self::Hertz => "Hz",
            Self::Kilohertz => "kHz",
        }
    }

    fn value(self, rate_hz: f32) -> f32 {
        match self {
            Self::Milliseconds => 1_000.0 / rate_hz,
            Self::Seconds => 1.0 / rate_hz,
            Self::Hertz => rate_hz,
            Self::Kilohertz => rate_hz / 1_000.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct NumericEntryState {
    target: NumericEntryTarget,
    draft: String,
    dirty: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum NumericEntryMessage {
    Begin {
        target: NumericEntryTarget,
    },
    DraftChanged {
        target: NumericEntryTarget,
        draft: String,
        dirty: bool,
    },
    Commit {
        target: NumericEntryTarget,
        draft: String,
    },
    Cancel {
        target: NumericEntryTarget,
    },
}

#[derive(Clone)]
struct RadiantEditorState {
    params: Arc<PumpParams>,
    status: Arc<GuiStatus>,
    host_param_edit_sink: Arc<dyn HostParamEditSink>,
    automation_config: AutomationConfig,
    #[cfg(test)]
    automation_flush_count: usize,
    active_curve_node: Option<usize>,
    active_curve_node_drag: Option<ActiveCurveNodeDrag>,
    active_curve_paint: Option<ActiveCurvePaint>,
    active_curve_segment: Option<ActiveCurveSegmentDrag>,
    active_curve_offset: Option<ActiveCurveOffsetDrag>,
    active_curve_marquee: Option<ActiveCurveMarquee>,
    selected_curve_nodes: Vec<usize>,
    preview_curve_offset: Option<EditableCurve>,
    hover_curve_node: Option<usize>,
    preview_curve_node: Option<CurveNode>,
    hover_curve_segment: Option<usize>,
    option_hover_held: bool,
    command_hover_held: bool,
    shift_hover_held: bool,
    loaded_global_curve_slot: Option<usize>,
    numeric_entry: Option<NumericEntryState>,
    active_knob_gesture: Option<NumericEntryTarget>,
    timing_dropdown_open: bool,
    hotkey_help_open: bool,
    free_rate_unit: FreeRateUnit,
    ab_confirmation: Option<String>,
    undo_history: Vec<RadiantHistorySnapshot>,
    redo_history: Vec<RadiantHistorySnapshot>,
}

#[derive(Clone)]
struct RadiantHistorySnapshot {
    mix: f32,
    smooth: f32,
    swing: f32,
    depth_db: f32,
    floor_db: f32,
    phase_offset: f32,
    output_gain_db: f32,
    sync_division: usize,
    mode: usize,
    curve: EditableCurve,
    active_sound: SoundSide,
    sound_states: [PumpSoundState; 2],
    stored_sound_states: [PumpSoundState; 2],
}

#[derive(Clone, Debug, PartialEq)]
enum RadiantEditorMessage {
    Undo,
    Redo,
    ToggleTimingMode,
    ToggleTimingDropdown,
    ToggleWaveformMode,
    ToggleHotkeyHelp,
    Knob {
        target: NumericEntryTarget,
        message: KnobMessage,
    },
    SyncDivision(f32),
    FreeRateUnit(FreeRateUnit),
    SelectSound {
        side: SoundSide,
        copy: bool,
    },
    CopyAndSelectSound(SoundSide),
    ToggleBypass,
    Curve(CurvePreviewMessage),
    CurveSlot(CurveSlotMessage),
    NumericEntry(NumericEntryMessage),
}

type EditorProjector = fn(&mut RadiantEditorState) -> Arc<UiSurface<RadiantEditorMessage>>;
type EditorReducer = fn(&mut RadiantEditorState, RadiantEditorMessage);
type EditorSurfaceRuntime = DeclarativeSurfaceRuntime<
    RadiantEditorState,
    RadiantEditorMessage,
    EditorProjector,
    EditorReducer,
>;

/// Host-controlled Radiant editor surface used by embedded Pump GUIs.
#[cfg(any(feature = "radiant-gui", feature = "vst3"))]
pub(crate) struct RadiantPumpEditor {
    runtime: EditorSurfaceRuntime,
    status: Arc<GuiStatus>,
    theme: ThemeTokens,
    paint_plan: SurfacePaintPlan,
    viewport: Vector2,
    bypass_revision: u32,
    bypass_automation_active: bool,
}

#[cfg(any(feature = "radiant-gui", feature = "vst3"))]
impl RadiantPumpEditor {
    /// Build a Radiant editor runtime at the provided logical viewport.
    pub(crate) fn new(
        params: Arc<PumpParams>,
        status: Arc<GuiStatus>,
        automation_queue: Arc<PumpAutomationQueue>,
        host_param_requester: Option<HostParamFlushRequester>,
        width: u32,
        height: u32,
    ) -> Self {
        Self::new_with_edit_sink(
            params,
            status,
            Arc::new(ClapHostParamEditSink {
                queue: automation_queue,
                requester: host_param_requester,
            }),
            width,
            height,
        )
    }

    /// Build a hosted editor with a format-specific UI-thread edit sink.
    pub(crate) fn new_with_edit_sink(
        params: Arc<PumpParams>,
        status: Arc<GuiStatus>,
        host_param_edit_sink: Arc<dyn HostParamEditSink>,
        width: u32,
        height: u32,
    ) -> Self {
        let theme = pump_theme();
        let viewport = Vector2::new(width.max(1) as f32, height.max(1) as f32);
        let bypass_revision = params.bypass_revision();
        let bypass_automation_active = params.bypass_automation_recent();
        Self {
            runtime: EditorSurfaceRuntime::new_declarative(
                RadiantEditorState::new(params, Arc::clone(&status), host_param_edit_sink),
                viewport,
                project_editor_surface,
                reduce_editor_message,
            ),
            status,
            paint_plan: SurfacePaintPlan::empty(&theme),
            theme,
            viewport,
            bypass_revision,
            bypass_automation_active,
        }
    }

    /// Apply a host-driven viewport resize.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.viewport = Vector2::new(width.max(1) as f32, height.max(1) as f32);
        self.runtime.dispatch_event(Event::resize(Vector2::new(
            width.max(1) as f32,
            height.max(1) as f32,
        )));
    }

    /// Route a backend-neutral event into the Radiant runtime.
    pub(crate) fn dispatch_event(&mut self, event: Event) {
        let _ = self.runtime.dispatch_event(event);
    }

    /// Route a focused key press into the Radiant runtime.
    pub(crate) fn dispatch_key_press(&mut self, key: WidgetKey) -> bool {
        self.runtime.dispatch_event(Event::key_press(key)).is_some()
    }

    /// Route a focused text character into the Radiant runtime.
    pub(crate) fn dispatch_character(&mut self, ch: char) -> bool {
        // Vim-style history shortcuts are only active when no numeric editor
        // owns text input. While an entry is focused, the character must keep
        // flowing through Radiant so normal editing semantics remain intact.
        if self.runtime.bridge().state().numeric_entry.is_none() {
            let message = match ch {
                'u' if !self.runtime.bridge().state().undo_history.is_empty() => {
                    Some(RadiantEditorMessage::Undo)
                }
                'U' if !self.runtime.bridge().state().redo_history.is_empty() => {
                    Some(RadiantEditorMessage::Redo)
                }
                _ => None,
            };
            if let Some(message) = message {
                self.runtime.dispatch_message(message);
                return true;
            }
        }
        self.runtime.dispatch_event(Event::character(ch)).is_some()
    }

    /// Cancel active numeric value entry, if any.
    pub(crate) fn cancel_numeric_entry(&mut self) -> bool {
        if self.runtime.bridge().state().numeric_entry.is_none() {
            return false;
        }
        let _ = self.runtime.dispatch_event(Event::clear_focus());
        if self.runtime.bridge().state().numeric_entry.is_some() {
            self.runtime.bridge_mut().state_mut().numeric_entry = None;
            self.runtime.refresh();
        }
        true
    }

    /// Return whether the hosted view should keep repainting without input.
    pub(crate) fn needs_realtime_redraw(&self) -> bool {
        self.status.has_host_beats_timeline()
            || self.status.is_playing()
            || self.status.gain_reduction_needs_redraw()
            || self
                .runtime
                .bridge()
                .state()
                .params
                .bypass_automation_recent()
            || self
                .runtime
                .bridge()
                .state()
                .params
                .has_pending_active_sound()
    }

    /// Refresh and return the current backend-neutral paint plan.
    pub(crate) fn paint_plan(&mut self) -> &SurfacePaintPlan {
        let bypass_revision = self.runtime.bridge().state().params.bypass_revision();
        let bypass_automation_active = self
            .runtime
            .bridge()
            .state()
            .params
            .bypass_automation_recent();
        if bypass_revision != self.bypass_revision
            || bypass_automation_active != self.bypass_automation_active
        {
            self.bypass_revision = bypass_revision;
            self.bypass_automation_active = bypass_automation_active;
            self.runtime.refresh();
        }
        if self.needs_realtime_redraw() {
            self.runtime.refresh();
        }
        let _ = self
            .runtime
            .borrowed_frame_into(&self.theme, &mut self.paint_plan);
        // The host owns window chrome; plugin content still receives a rounded
        // in-bounds frame so embedded surfaces retain the target's outer shell.
        self.paint_plan.primitives.push(PaintPrimitive::FillPath(
            PaintFillPath::new(
                0,
                rounded_frame_path(self.viewport),
                PaintBrush::solid(self.theme.border_emphasis),
            )
            .fill_rule(PaintFillRule::EvenOdd),
        ));
        &self.paint_plan
    }
}

fn rounded_frame_path(viewport: Vector2) -> PaintPath {
    let outer = Rect::from_xy_size(
        0.5,
        0.5,
        (viewport.x - 1.0).max(1.0),
        (viewport.y - 1.0).max(1.0),
    );
    let inner = outer.inset(1.0, 1.0, 1.0, 1.0);
    let radius = 7.65_f32.min(outer.width() * 0.5).min(outer.height() * 0.5);
    let inner_radius = (radius - 1.0).max(0.0);
    let mut commands = rounded_rect_commands(outer, radius);
    commands.extend(rounded_rect_commands(inner, inner_radius));
    PaintPath::from(commands)
}

fn rounded_rect_commands(rect: Rect, radius: f32) -> Vec<PaintPathCommand> {
    let left = rect.min.x;
    let right = rect.max.x;
    let top = rect.min.y;
    let bottom = rect.max.y;
    let r = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let k = 0.552_284_8 * r;
    vec![
        PaintPathCommand::MoveTo(Point::new(left + r, top)),
        PaintPathCommand::LineTo(Point::new(right - r, top)),
        PaintPathCommand::CurveTo {
            control1: Point::new(right - r + k, top),
            control2: Point::new(right, top + r - k),
            to: Point::new(right, top + r),
        },
        PaintPathCommand::LineTo(Point::new(right, bottom - r)),
        PaintPathCommand::CurveTo {
            control1: Point::new(right, bottom - r + k),
            control2: Point::new(right - r + k, bottom),
            to: Point::new(right - r, bottom),
        },
        PaintPathCommand::LineTo(Point::new(left + r, bottom)),
        PaintPathCommand::CurveTo {
            control1: Point::new(left + r - k, bottom),
            control2: Point::new(left, bottom - r + k),
            to: Point::new(left, bottom - r),
        },
        PaintPathCommand::LineTo(Point::new(left, top + r)),
        PaintPathCommand::CurveTo {
            control1: Point::new(left, top + r - k),
            control2: Point::new(left + r - k, top),
            to: Point::new(left + r, top),
        },
        PaintPathCommand::Close,
    ]
}

fn rounded_ring_path(rect: Rect, radius: f32, thickness: f32) -> PaintPath {
    let thickness = thickness.max(0.0);
    let inner = rect.inset(thickness, thickness, thickness, thickness);
    let outer_radius = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let inner_radius = (outer_radius - thickness)
        .min(inner.width() * 0.5)
        .min(inner.height() * 0.5)
        .max(0.0);
    let mut commands = rounded_rect_commands(rect, outer_radius);
    commands.extend(rounded_rect_commands(inner, inner_radius));
    PaintPath::from(commands)
}

#[cfg(any(feature = "radiant-gui", feature = "vst3"))]
impl toybox::radiant_gui::RadiantEditor for RadiantPumpEditor {
    fn resize(&mut self, width: u32, height: u32) {
        Self::resize(self, width, height);
    }

    fn dispatch_event(&mut self, event: Event) {
        Self::dispatch_event(self, event);
    }

    fn paint_plan(&mut self) -> &SurfacePaintPlan {
        Self::paint_plan(self)
    }

    fn needs_realtime_redraw(&self) -> bool {
        Self::needs_realtime_redraw(self)
    }

    fn dispatch_key_press(&mut self, key: WidgetKey) -> bool {
        Self::dispatch_key_press(self, key)
    }

    fn dispatch_character(&mut self, character: char) -> bool {
        Self::dispatch_character(self, character)
    }

    fn cancel_text_entry(&mut self) -> bool {
        Self::cancel_numeric_entry(self)
    }
}

/// Build one Radiant frame for tests and non-retained preview hosts.
#[cfg(test)]
pub(crate) fn radiant_editor_frame_for_params(
    params: Arc<PumpParams>,
    status: Arc<GuiStatus>,
    viewport: Vector2,
) -> SurfaceFrame {
    EditorSurfaceRuntime::new_declarative(
        RadiantEditorState::new(
            params,
            status,
            Arc::new(ClapHostParamEditSink {
                queue: Arc::new(PumpAutomationQueue::default()),
                requester: None,
            }),
        ),
        viewport,
        project_editor_surface,
        reduce_editor_message,
    )
    .frame(&pump_theme())
}

impl RadiantEditorState {
    fn new(
        params: Arc<PumpParams>,
        status: Arc<GuiStatus>,
        host_param_edit_sink: Arc<dyn HostParamEditSink>,
    ) -> Self {
        Self {
            params,
            status,
            host_param_edit_sink,
            automation_config: AutomationConfig::default(),
            #[cfg(test)]
            automation_flush_count: 0,
            active_curve_node: None,
            active_curve_node_drag: None,
            active_curve_paint: None,
            active_curve_segment: None,
            active_curve_offset: None,
            active_curve_marquee: None,
            selected_curve_nodes: Vec::new(),
            preview_curve_offset: None,
            hover_curve_node: None,
            preview_curve_node: None,
            hover_curve_segment: None,
            option_hover_held: false,
            command_hover_held: false,
            shift_hover_held: false,
            loaded_global_curve_slot: None,
            numeric_entry: None,
            active_knob_gesture: None,
            timing_dropdown_open: false,
            hotkey_help_open: false,
            free_rate_unit: FreeRateUnit::Hertz,
            ab_confirmation: None,
            undo_history: Vec::new(),
            redo_history: Vec::new(),
        }
    }

    fn snapshot(&self) -> RadiantHistorySnapshot {
        RadiantHistorySnapshot {
            mix: self.params.mix(),
            smooth: self.params.smooth(),
            swing: self.params.swing(),
            depth_db: self.params.depth_db(),
            floor_db: self.params.floor_db(),
            phase_offset: self.params.phase_offset(),
            output_gain_db: self.params.output_gain_db(),
            sync_division: self.params.sync_division(),
            mode: self.params.mode(),
            curve: self.params.editable_curve_snapshot(),
            active_sound: self.params.active_sound(),
            sound_states: [
                self.params.sound_state_snapshot(SoundSide::A),
                self.params.sound_state_snapshot(SoundSide::B),
            ],
            stored_sound_states: [
                self.params.stored_sound_state_snapshot(SoundSide::A),
                self.params.stored_sound_state_snapshot(SoundSide::B),
            ],
        }
    }

    fn push_history(&mut self) {
        self.push_history_snapshot(self.snapshot());
    }

    fn push_history_snapshot(&mut self, snapshot: RadiantHistorySnapshot) {
        self.ab_confirmation = None;
        self.undo_history.push(snapshot);
        if self.undo_history.len() > 128 {
            self.undo_history.remove(0);
        }
        self.redo_history.clear();
    }

    fn restore(&self, snapshot: &RadiantHistorySnapshot) {
        self.params.set_mix(snapshot.mix);
        self.params.set_smooth(snapshot.smooth);
        self.params.set_swing(snapshot.swing);
        self.params.set_depth_db(snapshot.depth_db);
        self.params.set_floor_db(snapshot.floor_db);
        self.params.set_phase_offset(snapshot.phase_offset);
        self.params.set_output_gain_db(snapshot.output_gain_db);
        self.params.set_sync_division(snapshot.sync_division as f32);
        self.params.set_mode(snapshot.mode as f32);
        self.params
            .set_editable_curve_preserving_phase(&snapshot.curve);
        self.params
            .set_sound_states_with_references_without_persistence(
                snapshot.active_sound,
                snapshot.sound_states.clone(),
                snapshot.stored_sound_states.clone(),
            );
    }

    fn undo(&mut self) {
        if let Some(snapshot) = self.undo_history.pop() {
            self.redo_history.push(self.snapshot());
            self.restore(&snapshot);
            self.clear_curve_selection();
        }
    }

    fn redo(&mut self) {
        if let Some(snapshot) = self.redo_history.pop() {
            self.undo_history.push(self.snapshot());
            self.restore(&snapshot);
            self.clear_curve_selection();
        }
    }

    fn clear_curve_selection(&mut self) {
        self.active_curve_marquee = None;
        self.selected_curve_nodes.clear();
    }
}

#[allow(clippy::arc_with_non_send_sync)]
fn project_editor_surface(state: &mut RadiantEditorState) -> Arc<UiSurface<RadiantEditorMessage>> {
    if state.params.consume_pending_active_sound().is_some() {
        state.clear_curve_selection();
    }
    let params = state.params.as_ref();
    let curve = state
        .preview_curve_offset
        .clone()
        .unwrap_or_else(|| params.editable_curve_snapshot());
    let output = params.output_gain_db();
    let smooth = params.smooth();
    let swing = params.swing();
    let depth = params.depth_db();
    let floor = params.floor_db();
    let sync = params.sync_division();
    let free_timing = params.timing_mode() == TIMING_MODE_FREE;
    let free_rate = params.free_rate_hz();
    let waveform_live_mode = state.status.waveform_live_mode();
    let playhead_phase = (state.status.has_host_beats_timeline() || state.status.is_playing())
        .then_some(state.status.phase());
    let active_sound = params.active_sound();
    let timing_options: Vec<_> = if free_timing {
        FreeRateUnit::ALL
            .into_iter()
            .map(|unit| {
                DropdownOption::new(
                    unit.label(),
                    unit == state.free_rate_unit,
                    RadiantEditorMessage::FreeRateUnit(unit),
                )
            })
            .collect()
    } else {
        SYNC_DIVISIONS
            .iter()
            .enumerate()
            .map(|(index, division)| {
                DropdownOption::new(
                    division.label,
                    index == sync,
                    RadiantEditorMessage::SyncDivision(normalize_sync_division(index)),
                )
            })
            .collect()
    };
    let timing_label = if free_timing {
        state.free_rate_unit.label().to_string()
    } else {
        format!("Sync {}", sync_division_label(sync))
    };
    let timing_dropdown = dropdown_trigger(timing_label, state.timing_dropdown_open)
        .toggle_message(RadiantEditorMessage::ToggleTimingDropdown)
        .build();
    let timing_toggle = toggle(if free_timing { "FREE" } else { "SYNC" }, free_timing)
        .message(|_| RadiantEditorMessage::ToggleTimingMode)
        .size(TIMING_MODE_TOGGLE_WIDTH, TIMING_CONTROL_HEIGHT)
        .tooltip(if free_timing {
            "Switch to synchronized timing"
        } else {
            "Switch to free timing"
        });
    let timing_header_controls = row([
        timing_toggle,
        timing_dropdown
            .width(TIMING_DROPDOWN_WIDTH)
            .height(TIMING_CONTROL_HEIGHT),
    ])
    .spacing(PUMP_VISUAL_METRICS.space_4)
    .height(TIMING_CONTROL_HEIGHT);
    let history_actions = row([
        action_icon_button_with_state(
            IconName::ChevronLeft,
            "Undo",
            RadiantEditorMessage::Undo,
            PUMP_VISUAL_METRICS.icon_hit,
            TIMING_CONTROL_HEIGHT,
            state.undo_history.is_empty(),
        ),
        action_icon_button_with_state(
            IconName::ChevronRight,
            "Redo",
            RadiantEditorMessage::Redo,
            PUMP_VISUAL_METRICS.icon_hit,
            TIMING_CONTROL_HEIGHT,
            state.redo_history.is_empty(),
        ),
    ])
    .spacing(PUMP_VISUAL_METRICS.space_4)
    .height(TIMING_CONTROL_HEIGHT);
    let ab_actions = row([
        custom_widget_direct(SoundSideButtonWidget::new(
            SoundSide::A,
            active_sound == SoundSide::A,
        ))
        .size(28.9, TIMING_CONTROL_HEIGHT)
        .tooltip("Select sound A"),
        custom_widget_direct(SoundSwitchButtonWidget::new(active_sound))
            .size(PUMP_VISUAL_METRICS.icon_hit, TIMING_CONTROL_HEIGHT)
            .tooltip(if active_sound == SoundSide::A {
                "Switch to sound B; Cmd-click copies A to B"
            } else {
                "Switch to sound A; Cmd-click copies B to A"
            }),
        custom_widget_direct(SoundSideButtonWidget::new(
            SoundSide::B,
            active_sound == SoundSide::B,
        ))
        .size(28.9, TIMING_CONTROL_HEIGHT)
        .tooltip("Select sound B"),
    ])
    .spacing(PUMP_VISUAL_METRICS.space_4)
    .height(TIMING_CONTROL_HEIGHT);
    let hotkey_help_action =
        custom_widget_mapped(HotkeyHelpButtonWidget::new(), move |_: ButtonMessage| {
            RadiantEditorMessage::ToggleHotkeyHelp
        })
        .size(28.0, 28.0)
        .tooltip("Show hotkeys");
    let header_brand = column([
        row([
            text("PORTALSURFER")
                .muted_text()
                .width(HEADER_BRAND_WORDMARK_WIDTH)
                .align_text(TextAlign::Right),
            text("/")
                .muted_text()
                .width(8.5)
                .align_text(TextAlign::Center),
            text("PUMP")
                .text_color(TextColorRole::Custom(pump_theme().accent_copper))
                .align_text(TextAlign::Right)
                .width(35.7),
        ])
        .align_main(MainAlign::End)
        .fill_width()
        .height(HEADER_BRAND_TITLE_HEIGHT),
        custom_widget(
            MetadataTextWidget::new(if params.preset_persistence_warning().is_some() {
                super::PRESET_WARNING_STORAGE.to_string()
            } else {
                build_version_label()
            }),
            |_| None,
        )
        .fill_width()
        .height(HEADER_BRAND_META_HEIGHT),
    ])
    .spacing(0.0)
    .width(HEADER_BRAND_WIDTH)
    .height(BUILD_LABEL_HEIGHT);
    let header_actions = row([
        timing_header_controls,
        history_actions,
        ab_actions,
        spacer().fill_width(),
        header_brand,
        hotkey_help_action,
    ])
    .spacing(PUMP_VISUAL_METRICS.gap)
    .align_cross(CrossAlign::Center)
    .fill_width()
    .height(BUILD_LABEL_HEIGHT);
    let base = column([
        header_actions,
        spacer().height(HEADER_TO_CURVE_GAP),
        row([
            custom_widget_mapped(
                CurvePreviewWidget::new(
                    curve,
                    state.active_curve_node,
                    state.active_curve_segment.as_ref().map(|drag| drag.index),
                    state.hover_curve_node,
                    state.preview_curve_node,
                    state.hover_curve_segment,
                    state.option_hover_held,
                )
                .with_command_hover_held(state.command_hover_held)
                .with_shift_hover_held(state.shift_hover_held)
                .with_active_curve_paint(state.active_curve_paint.is_some())
                .with_curve_paint_runs(
                    state
                        .active_curve_paint
                        .as_ref()
                        .map(ActiveCurvePaint::preview_runs),
                )
                .with_active_segment_move(
                    state
                        .active_curve_segment
                        .as_ref()
                        .is_some_and(|drag| drag.mode == CurveSegmentDragMode::MovePair),
                )
                .with_active_curve_offset(
                    state
                        .active_curve_offset
                        .as_ref()
                        .map(|drag| drag.start_pointer_x),
                )
                .with_selected_curve_nodes(&state.selected_curve_nodes)
                .with_active_curve_marquee(state.active_curve_marquee)
                .with_incoming_waveform(state.status.incoming_waveform_snapshot())
                .with_sync_division(sync)
                .with_swing(swing)
                .with_smooth(smooth)
                .with_phase_offset(state.params.phase_offset())
                .with_gain_mapping(depth, floor)
                .with_playhead_phase(playhead_phase),
                RadiantEditorMessage::Curve,
            )
            .fill_width()
            .fill_height(),
            custom_widget(
                GainReductionMeterWidget::new(state.status.gain_reduction_db()),
                |_| None,
            )
            .width(GAIN_REDUCTION_METER_WIDTH)
            .fill_height(),
        ])
        .spacing(CURVE_METER_GAP)
        .fill_width()
        .fill_height(),
        curve_slot_row(state),
        parameter_deck(
            state,
            params,
            output,
            smooth,
            free_timing,
            free_rate,
            state.free_rate_unit,
        ),
        row([
            toggle(
                if waveform_live_mode { "LIVE" } else { "SYNC" },
                waveform_live_mode,
            )
            .message(|_| RadiantEditorMessage::ToggleWaveformMode)
            .size(WAVEFORM_MODE_CONTROL_WIDTH, CONTROL_ROW_HEIGHT)
            .tooltip("Live waveform replacement; off holds each completed cycle"),
            spacer().fill_width(),
            custom_widget_mapped(
                BypassControlWidget::new(params.bypassed(), params.bypass_automation_recent())
                    .with_pulse_phase(state.status.is_playing().then_some(state.status.phase())),
                move |_: ButtonMessage| RadiantEditorMessage::ToggleBypass,
            )
            .width(BYPASS_CONTROL_WIDTH)
            .height(CONTROL_ROW_HEIGHT),
        ])
        .spacing(PUMP_VISUAL_METRICS.space_4)
        .fill_width()
        .height(CONTROL_ROW_HEIGHT),
    ])
    .padding(SURFACE_PADDING)
    .spacing(SURFACE_SPACING)
    .fill_height();
    let surface = if state.timing_dropdown_open {
        dismissible_overlay(
            base,
            dropdown_menu_overlay_below(
                SURFACE_PADDING + TIMING_MODE_TOGGLE_WIDTH + PUMP_VISUAL_METRICS.space_4,
                SURFACE_PADDING,
                TIMING_CONTROL_HEIGHT,
                SURFACE_SPACING,
                Some(TIMING_DROPDOWN_WIDTH),
                timing_options,
            ),
            RadiantEditorMessage::ToggleTimingDropdown,
        )
    } else if state.hotkey_help_open {
        dismissible_overlay(
            base,
            hotkey_help_overlay(),
            RadiantEditorMessage::ToggleHotkeyHelp,
        )
    } else {
        // Keep the base content under a stable stack root whether a transient
        // overlay is open or not. This preserves widget identities (and focus)
        // when the help panel is toggled.
        stack([base]).fill()
    };
    Arc::new(surface.into_surface())
}

fn hotkey_help_overlay() -> ViewNode<RadiantEditorMessage> {
    column([
        spacer()
            .height(SURFACE_PADDING + BUILD_LABEL_HEIGHT + SURFACE_SPACING)
            .fill_width(),
        row([
            spacer().fill_width(),
            custom_widget(HotkeyHelpWidget::new(), |_| None)
                .width(HOTKEY_HELP_WIDTH)
                .height(HOTKEY_HELP_HEIGHT),
            spacer().width(SURFACE_PADDING),
        ])
        .fill_width()
        .height(HOTKEY_HELP_HEIGHT),
        spacer().fill_height(),
    ])
    .fill()
}

fn parameter_deck(
    state: &RadiantEditorState,
    params: &PumpParams,
    output: f32,
    smooth: f32,
    free_timing: bool,
    free_rate: f32,
    free_rate_unit: FreeRateUnit,
) -> ViewNode<RadiantEditorMessage> {
    let mut controls = vec![
        control_column(
            NumericEntryTarget::Smooth,
            "SMOOTH",
            format_plain_value_text(PARAM_SMOOTH_ID, smooth as f64)
                .unwrap_or_else(|| format!("{smooth:.0}%")),
            smooth,
            DEFAULT_SMOOTH,
            state.numeric_entry.as_ref(),
        ),
        parameter_deck_divider(),
        control_column(
            NumericEntryTarget::Swing,
            "SWING",
            format_plain_value_text(PARAM_SWING_ID, params.swing() as f64)
                .unwrap_or_else(|| format!("{:.0}%", params.swing() * 100.0)),
            params.swing(),
            crate::params::DEFAULT_SWING,
            state.numeric_entry.as_ref(),
        ),
    ];
    if free_timing {
        let normalized =
            normalized_from_plain_value(PARAM_FREE_RATE_ID, free_rate as f64).unwrap_or(0.0) as f32;
        controls.push(parameter_deck_divider());
        controls.push(control_column(
            NumericEntryTarget::FreeRate,
            "RATE",
            format_free_rate_for_unit(free_rate, free_rate_unit),
            normalized,
            normalized_from_plain_value(PARAM_FREE_RATE_ID, DEFAULT_FREE_RATE_HZ as f64)
                .map(|value| value as f32)
                .unwrap_or(0.0),
            state.numeric_entry.as_ref(),
        ));
    }
    controls.extend([
        parameter_deck_divider(),
        control_column(
            NumericEntryTarget::Mix,
            "MIX",
            format!("{:.0}%", params.mix() * 100.0),
            params.mix(),
            DEFAULT_MIX,
            state.numeric_entry.as_ref(),
        ),
        control_column(
            NumericEntryTarget::OutputGain,
            "OUTPUT",
            format!("{output:+.1} dB"),
            normalize_output_gain(output),
            normalize_output_gain(DEFAULT_OUTPUT_GAIN_DB),
            state.numeric_entry.as_ref(),
        ),
        parameter_deck_divider(),
    ]);
    row(controls)
        .spacing(PUMP_VISUAL_METRICS.gap)
        .fill_width()
        .height(PARAMETER_DECK_HEIGHT)
}

fn parameter_deck_divider() -> ViewNode<RadiantEditorMessage> {
    custom_widget(ParameterDeckDividerWidget::new(), |_| None)
        .width(PUMP_VISUAL_METRICS.divider)
        .height(PARAMETER_DECK_HEIGHT)
}

fn knob_control(
    target: NumericEntryTarget,
    label: &'static str,
    value_label: String,
    value: f32,
    default_value: Option<f32>,
    active_entry: Option<&NumericEntryState>,
    height: f32,
) -> ViewNode<RadiantEditorMessage> {
    let knob_diameter = (height - 2.0 * CONTROL_ROW_HEIGHT - 2.0 * PUMP_VISUAL_METRICS.space_4)
        .clamp(13.6, PUMP_VISUAL_METRICS.knob);
    let mut knob_builder = knob(value.clamp(0.0, 1.0))
        .diameter(knob_diameter)
        .primary();
    if let Some(default_value) = default_value {
        knob_builder = knob_builder.default_value(default_value.clamp(0.0, 1.0));
    }
    let knob_view = knob_builder
        .message(move |message| RadiantEditorMessage::Knob { target, message })
        .width(knob_diameter)
        .height(knob_diameter);
    column([
        text(label)
            .fill_width()
            .height(CONTROL_ROW_HEIGHT)
            .align_text(TextAlign::Center),
        knob_view,
        value_label_node(target, value_label, active_entry),
    ])
    .spacing(PUMP_VISUAL_METRICS.space_4)
    .fill_width()
    .align_cross(CrossAlign::Center)
    .height(height)
}

fn control_column(
    target: NumericEntryTarget,
    label: &'static str,
    value_label: String,
    value: f32,
    default_value: f32,
    active_entry: Option<&NumericEntryState>,
) -> ViewNode<RadiantEditorMessage> {
    knob_control(
        target,
        label,
        value_label,
        value,
        Some(default_value),
        active_entry,
        PARAMETER_DECK_HEIGHT,
    )
}

fn reduce_knob_message(
    state: &mut RadiantEditorState,
    target: NumericEntryTarget,
    message: KnobMessage,
) {
    match message {
        KnobMessage::GestureStarted { .. } => {
            if state.active_knob_gesture.is_none()
                && state
                    .host_param_edit_sink
                    .gesture_started(&state.automation_config, target.param_id())
            {
                state.push_history();
                state.active_knob_gesture = Some(target);
            }
        }
        KnobMessage::ValueChanged { value } => {
            if state.active_knob_gesture == Some(target) {
                let (param_id, plain_value) = knob_plain_value(target, value);
                if !state.host_param_edit_sink.gesture_value(
                    &state.automation_config,
                    param_id,
                    plain_value as f64,
                ) {
                    return;
                }
                let _ = set_knob_param(state.params.as_ref(), target, value);
            }
        }
        KnobMessage::GestureEnded { .. } => {
            if state.active_knob_gesture == Some(target) {
                let _ = state
                    .host_param_edit_sink
                    .gesture_ended(&state.automation_config, target.param_id());
                state.active_knob_gesture = None;
            }
        }
        KnobMessage::Reset { value } => {
            if state.active_knob_gesture == Some(target) {
                let _ = state
                    .host_param_edit_sink
                    .gesture_ended(&state.automation_config, target.param_id());
                state.active_knob_gesture = None;
            }
            let (param_id, plain_value) = knob_plain_value(target, value);
            if state.host_param_edit_sink.edit(
                &state.automation_config,
                param_id,
                plain_value as f64,
            ) {
                state.push_history();
                let _ = set_knob_param(state.params.as_ref(), target, value);
            }
        }
        KnobMessage::KeyboardGesture(gesture) => {
            reduce_discrete_knob_gesture(state, target, gesture.events);
        }
        KnobMessage::WheelGesture(gesture) => {
            reduce_discrete_knob_gesture(state, target, gesture.events);
        }
    }
}

fn reduce_discrete_knob_gesture(
    state: &mut RadiantEditorState,
    target: NumericEntryTarget,
    events: [radiant::prelude::KnobAutomationEvent; 3],
) {
    let Some(value) = (match events[1] {
        radiant::prelude::KnobAutomationEvent::ValueChanged { value } => Some(value),
        _ => None,
    }) else {
        return;
    };
    if !state
        .host_param_edit_sink
        .gesture_started(&state.automation_config, target.param_id())
    {
        return;
    }
    state.push_history();
    let (param_id, plain_value) = knob_plain_value(target, value);
    if state.host_param_edit_sink.gesture_value(
        &state.automation_config,
        param_id,
        plain_value as f64,
    ) {
        let _ = set_knob_param(state.params.as_ref(), target, value);
    }
    let _ = state
        .host_param_edit_sink
        .gesture_ended(&state.automation_config, target.param_id());
}

fn knob_plain_value(target: NumericEntryTarget, value: f32) -> (ClapId, f32) {
    match target {
        NumericEntryTarget::Mix => (PARAM_MIX_ID, value),
        NumericEntryTarget::OutputGain => (PARAM_OUTPUT_GAIN_ID, denormalize_output_gain(value)),
        NumericEntryTarget::Smooth => (PARAM_SMOOTH_ID, value),
        NumericEntryTarget::Swing => (PARAM_SWING_ID, value),
        NumericEntryTarget::FreeRate => (
            PARAM_FREE_RATE_ID,
            plain_from_normalized_value(PARAM_FREE_RATE_ID, value as f64)
                .unwrap_or(DEFAULT_FREE_RATE_HZ as f64) as f32,
        ),
    }
}

fn set_knob_param(params: &PumpParams, target: NumericEntryTarget, value: f32) -> (ClapId, f32) {
    let (param_id, plain_value) = knob_plain_value(target, value);
    match target {
        NumericEntryTarget::Mix => params.set_mix(value),
        NumericEntryTarget::OutputGain => params.set_output_gain_db(plain_value),
        NumericEntryTarget::Smooth => params.set_smooth(value),
        NumericEntryTarget::Swing => params.set_swing(value),
        NumericEntryTarget::FreeRate => params.set_free_rate_hz(plain_value),
    }
    (param_id, plain_value)
}

fn format_free_rate_for_unit(rate_hz: f32, unit: FreeRateUnit) -> String {
    let value = unit.value(rate_hz);
    match unit {
        FreeRateUnit::Milliseconds => format!("{value:.1} ms"),
        FreeRateUnit::Seconds => format!("{value:.3} s"),
        FreeRateUnit::Hertz => format!("{value:.2} Hz"),
        FreeRateUnit::Kilohertz => format!("{value:.3} kHz"),
    }
}

fn value_label_node(
    target: NumericEntryTarget,
    value_label: String,
    active_entry: Option<&NumericEntryState>,
) -> ViewNode<RadiantEditorMessage> {
    let (display, editing, dirty) = active_entry
        .filter(|entry| entry.target == target)
        .map(|entry| (entry.draft.clone(), true, entry.dirty))
        .unwrap_or((value_label, false, false));
    custom_widget_mapped(
        NumericValueLabelWidget::new(target, display, editing, dirty),
        RadiantEditorMessage::NumericEntry,
    )
    .key(target.widget_key())
    .width(CONTROL_VALUE_WIDTH)
    .height(CONTROL_ROW_HEIGHT)
}

fn reduce_editor_message(state: &mut RadiantEditorState, message: RadiantEditorMessage) {
    match message {
        RadiantEditorMessage::Undo => {
            state.active_curve_paint = None;
            state.undo();
        }
        RadiantEditorMessage::Redo => {
            state.active_curve_paint = None;
            state.redo();
        }
        RadiantEditorMessage::ToggleTimingMode => {
            state.push_history();
            state.timing_dropdown_open = false;
            let timing_mode = if state.params.timing_mode() == TIMING_MODE_FREE {
                TIMING_MODE_SYNC
            } else {
                TIMING_MODE_FREE
            };
            state.params.set_timing_mode(timing_mode as f32);
            push_radiant_param_update(state, PARAM_TIMING_MODE_ID, timing_mode as f64);
        }
        RadiantEditorMessage::ToggleTimingDropdown => {
            state.timing_dropdown_open = !state.timing_dropdown_open;
        }
        RadiantEditorMessage::ToggleWaveformMode => {
            state
                .status
                .set_waveform_live_mode(!state.status.waveform_live_mode());
        }
        RadiantEditorMessage::ToggleHotkeyHelp => {
            state.hotkey_help_open = !state.hotkey_help_open;
            if state.hotkey_help_open {
                state.timing_dropdown_open = false;
            }
        }
        RadiantEditorMessage::Knob { target, message } => {
            reduce_knob_message(state, target, message);
        }
        RadiantEditorMessage::SyncDivision(value) => {
            state.push_history();
            state.timing_dropdown_open = false;
            let value = (value.clamp(0.0, 1.0) * MAX_SYNC_DIVISION).round();
            state.params.set_sync_division(value);
            push_radiant_param_update(state, PARAM_SYNC_DIVISION_ID, value as f64);
        }
        RadiantEditorMessage::FreeRateUnit(unit) => {
            state.free_rate_unit = unit;
            state.timing_dropdown_open = false;
        }
        RadiantEditorMessage::SelectSound { side, copy } => {
            if copy {
                let active = state.params.active_sound();
                if side == active.other() {
                    let before = state.snapshot();
                    if state.params.copy_active_to_inactive() {
                        state.push_history_snapshot(before);
                        state.clear_curve_selection();
                        state.ab_confirmation =
                            Some(format!("Copied {} → {}", active.label(), side.label()));
                    }
                }
                return;
            }
            if state.params.active_sound() != side {
                state.push_history();
                if state.params.set_active_sound(side) {
                    state.clear_curve_selection();
                    state.ab_confirmation = Some(format!("Switched to sound {}", side.label()));
                    push_radiant_param_update(state, PARAM_SOUND_ID, side.index() as f64);
                }
            }
        }
        RadiantEditorMessage::CopyAndSelectSound(side) => {
            let active = state.params.active_sound();
            if active == side {
                return;
            }

            let before = state.snapshot();
            let copied = state.params.copy_active_to_inactive();
            if !state.params.set_active_sound(side) {
                return;
            }

            state.push_history_snapshot(before);
            state.clear_curve_selection();
            state.ab_confirmation = Some(if copied {
                format!(
                    "Copied {} → {}; switched to sound {}",
                    active.label(),
                    side.label(),
                    side.label()
                )
            } else {
                format!("Switched to sound {}", side.label())
            });
            push_radiant_param_update(state, PARAM_SOUND_ID, side.index() as f64);
        }
        RadiantEditorMessage::ToggleBypass => {
            if try_toggle_bypass(
                state.params.as_ref(),
                state.host_param_edit_sink.as_ref(),
                &state.automation_config,
            ) {
                #[cfg(test)]
                {
                    state.automation_flush_count += 1;
                }
            }
        }
        RadiantEditorMessage::Curve(message) => {
            if matches!(
                message,
                CurvePreviewMessage::PressNode { .. }
                    | CurvePreviewMessage::PressCurveOffset { .. }
                    | CurvePreviewMessage::ResetCurveOffset
                    | CurvePreviewMessage::InsertNode { .. }
                    | CurvePreviewMessage::DeleteNode { .. }
                    | CurvePreviewMessage::DeleteSelectedNodes
                    | CurvePreviewMessage::PressSegment { .. }
                    | CurvePreviewMessage::PressSegmentMove { .. }
            ) {
                state.push_history();
            }
            reduce_curve_message(state, message)
        }
        RadiantEditorMessage::CurveSlot(message) => reduce_curve_slot_message(state, message),
        RadiantEditorMessage::NumericEntry(message) => reduce_numeric_entry_message(state, message),
    }
}

fn curve_slot_row(state: &RadiantEditorState) -> ViewNode<RadiantEditorMessage> {
    let slots = state.params.global_curve_slots_snapshot();
    let loaded_slot = state.loaded_global_curve_slot;
    let deviated_slot =
        loaded_slot.filter(|index| state.params.current_curve_deviates_from_global_slot(*index));
    let slot_nodes: Vec<ViewNode<RadiantEditorMessage>> = (0..GLOBAL_CURVE_SLOT_COUNT)
        .map(|index| {
            let curve = slots.get(index).and_then(|slot| slot.curve.clone());
            custom_widget_mapped(
                CurveSlotWidget::new(
                    index,
                    curve,
                    loaded_slot == Some(index),
                    deviated_slot == Some(index),
                ),
                RadiantEditorMessage::CurveSlot,
            )
            .fill_width()
            .height(CURVE_SLOT_ROW_HEIGHT)
        })
        .collect();
    row(slot_nodes)
        .spacing(CURVE_SLOT_SPACING)
        .fill_width()
        .height(CURVE_SLOT_ROW_HEIGHT)
}

fn reduce_curve_slot_message(state: &mut RadiantEditorState, message: CurveSlotMessage) {
    match message {
        CurveSlotMessage::Load { index } => {
            let Some(curve) = state.params.global_curve_slot_curve(index) else {
                return;
            };
            state.params.set_editable_curve_preserving_phase(&curve);
            state.clear_curve_selection();
            state.active_curve_node = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            state.loaded_global_curve_slot = Some(index);
        }
        CurveSlotMessage::Store { index } => {
            let curve = state.params.editable_curve_snapshot();
            if state.params.set_global_curve_slot_curve(index, &curve) {
                state.loaded_global_curve_slot = Some(index);
            }
        }
    }
}

fn reduce_numeric_entry_message(state: &mut RadiantEditorState, message: NumericEntryMessage) {
    match message {
        NumericEntryMessage::Begin { target } => {
            let value = target.current_plain_value(state.params.as_ref());
            let draft = if target == NumericEntryTarget::FreeRate {
                format_free_rate_for_unit(value as f32, state.free_rate_unit)
            } else {
                format_plain_value_text(target.param_id(), value)
                    .unwrap_or_else(|| value.to_string())
            };
            state.numeric_entry = Some(NumericEntryState {
                target,
                draft,
                dirty: false,
            });
        }
        NumericEntryMessage::DraftChanged {
            target,
            draft,
            dirty,
        } => {
            if state
                .numeric_entry
                .as_ref()
                .is_some_and(|entry| entry.target == target)
            {
                state.numeric_entry = Some(NumericEntryState {
                    target,
                    draft,
                    dirty,
                });
            }
        }
        NumericEntryMessage::Commit { target, draft } => {
            let entry_active = state
                .numeric_entry
                .as_ref()
                .is_some_and(|entry| entry.target == target);
            if entry_active {
                let Some(value) = parse_plain_value_text(target.param_id(), draft.trim()) else {
                    return;
                };
                apply_numeric_entry_value(state, target, value);
                state.numeric_entry = None;
            }
        }
        NumericEntryMessage::Cancel { target } => {
            if state
                .numeric_entry
                .as_ref()
                .is_some_and(|entry| entry.target == target)
            {
                state.numeric_entry = None;
            }
        }
    }
}

fn apply_numeric_entry_value(
    state: &mut RadiantEditorState,
    target: NumericEntryTarget,
    value: f64,
) {
    match target {
        NumericEntryTarget::Mix => state.params.set_mix(value as f32),
        NumericEntryTarget::OutputGain => state.params.set_output_gain_db(value as f32),
        NumericEntryTarget::Smooth => state.params.set_smooth(value as f32),
        NumericEntryTarget::Swing => state.params.set_swing(value as f32),
        NumericEntryTarget::FreeRate => state.params.set_free_rate_hz(value as f32),
    }

    push_radiant_param_update(state, target.param_id(), value);
}

/// Queue one complete discrete gesture and flush it once.
///
/// Pointer drags currently arrive as discrete reducer messages, so each update
/// is represented by its own begin/value/end batch. A future pointer-stream
/// reducer can coalesce these into one host gesture without changing the queue
/// contract here.
fn push_radiant_param_update(state: &mut RadiantEditorState, param_id: ClapId, value: f64) {
    if state
        .host_param_edit_sink
        .edit(&state.automation_config, param_id, value)
    {
        #[cfg(test)]
        {
            state.automation_flush_count += 1;
        }
    }
}

/// Ask the host to accept the next bypass value before changing shared state.
pub(crate) fn try_toggle_bypass(
    params: &PumpParams,
    sink: &dyn HostParamEditSink,
    config: &AutomationConfig,
) -> bool {
    let value = if params.bypassed() {
        BYPASS_ACTIVE_VALUE
    } else {
        BYPASS_BYPASSED_VALUE
    };
    if !sink.edit(config, PARAM_BYPASS_ID, value as f64) {
        return false;
    }
    params.set_bypass(value);
    true
}

fn reduce_curve_message(state: &mut RadiantEditorState, message: CurvePreviewMessage) {
    match message {
        CurvePreviewMessage::Hover {
            node,
            preview_node,
            segment,
        } => {
            state.hover_curve_node = node;
            state.preview_curve_node = preview_node;
            state.hover_curve_segment = segment;
        }
        CurvePreviewMessage::ModifiersChanged {
            option_held,
            command_held,
            shift_held,
        } => {
            let command_released = state.command_hover_held && !command_held;
            let constraint_changed =
                state.shift_hover_held != shift_held || state.option_hover_held != option_held;
            state.option_hover_held = option_held;
            state.command_hover_held = command_held;
            state.shift_hover_held = shift_held;
            if let Some(drag) = state.active_curve_offset.as_mut() {
                drag.quantized = option_held;
                let raw_delta = drag.raw_delta;
                let phase_offset = resolve_curve_offset(
                    state.params.sync_division(),
                    (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0),
                    state.params.swing(),
                    drag.origin_phase_offset,
                    raw_delta,
                    drag.quantized,
                );
                if state.host_param_edit_sink.gesture_value(
                    &state.automation_config,
                    PARAM_PHASE_OFFSET_ID,
                    phase_offset as f64,
                ) {
                    state.params.set_phase_offset(phase_offset);
                }
            }
            if constraint_changed {
                let curve = state.params.editable_curve_snapshot();
                let active_index = state.active_curve_node;
                if let Some(drag) = state.active_curve_node_drag.as_mut() {
                    let current = active_index
                        .and_then(|index| curve.nodes.get(index))
                        .copied()
                        .unwrap_or_else(|| drag.origin_curve.nodes[drag.origin_index]);
                    drag.set_constraints(shift_held, option_held, current);
                }
            }
            if option_held || command_held {
                state.preview_curve_node = None;
            }
            if command_released {
                if state
                    .active_curve_segment
                    .as_ref()
                    .is_some_and(|drag| drag.mode == CurveSegmentDragMode::MovePair)
                {
                    state.active_curve_segment = None;
                }
                state.hover_curve_segment = None;
            }
        }
        CurvePreviewMessage::PressPaint { sample } => {
            let mut paint = ActiveCurvePaint::new(state.snapshot(), state.params.phase_offset());
            paint.push_sample(sample);
            state.clear_curve_selection();
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.active_curve_marquee = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            state.active_curve_paint = Some(paint);
        }
        CurvePreviewMessage::DragPaint { sample } => {
            if let Some(paint) = state.active_curve_paint.as_mut() {
                paint.push_sample(sample);
            }
        }
        CurvePreviewMessage::DragPaintOutside { sample } => {
            if let Some(paint) = state.active_curve_paint.as_mut() {
                paint.push_boundary_sample(sample);
            }
        }
        CurvePreviewMessage::ReleasePaint { sample } => {
            if let Some(mut paint) = state.active_curve_paint.take() {
                if let Some(sample) = sample {
                    paint.push_sample(sample);
                }
                commit_active_curve_paint(state, paint);
            }
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.active_curve_marquee = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::ReleasePaintOutside { sample } => {
            if let Some(mut paint) = state.active_curve_paint.take() {
                paint.push_boundary_sample(sample);
                commit_active_curve_paint(state, paint);
            }
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.active_curve_marquee = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::PressNode {
            index,
            pointer,
            shift_held,
            option_held,
            command_held,
        } => {
            let curve = state.params.editable_curve_snapshot();
            let survivors = interactive_curve_node_survivors(
                &curve,
                state.params.phase_offset(),
                state.active_curve_node,
            );
            if !survivors.contains(&index) {
                state.clear_curve_selection();
                return;
            }
            let selected_indices = if state.selected_curve_nodes.contains(&index) {
                state
                    .selected_curve_nodes
                    .iter()
                    .copied()
                    .filter(|selected| survivors.contains(selected))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            if selected_indices.is_empty() {
                state.clear_curve_selection();
            }
            if let Some(mut drag) =
                start_curve_node_drag(&curve, index, pointer, shift_held, option_held)
            {
                drag.selected_indices = selected_indices;
                state.option_hover_held = option_held;
                state.command_hover_held = command_held;
                state.active_curve_node = Some(index);
                state.active_curve_node_drag = Some(drag);
                state.active_curve_paint = None;
                state.active_curve_segment = None;
                state.active_curve_offset = None;
                state.preview_curve_offset = None;
                state.shift_hover_held = shift_held;
                state.hover_curve_node = Some(index);
                state.preview_curve_node = None;
                state.hover_curve_segment = None;
            }
        }
        CurvePreviewMessage::PressMarquee { start } => {
            state.active_curve_marquee = Some(ActiveCurveMarquee {
                start,
                current: start,
            });
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::DragMarquee { current } => {
            if let Some(marquee) = state.active_curve_marquee.as_mut() {
                marquee.current = current;
            }
        }
        CurvePreviewMessage::ReleaseMarquee { current } => {
            let Some(marquee) = state.active_curve_marquee.take() else {
                return;
            };
            let min_y = marquee.start.y.min(current.y);
            let max_y = marquee.start.y.max(current.y);
            let curve = state.params.editable_curve_snapshot();
            let phase_offset = state.params.phase_offset();
            let survivors = interactive_curve_node_survivors(&curve, phase_offset, None);
            let start_x = CurvePreviewWidget::display_phase(marquee.start.x, phase_offset);
            let current_x = CurvePreviewWidget::display_phase(current.x, phase_offset);
            let min_display_x = start_x.min(current_x);
            let max_display_x = start_x.max(current_x);
            state.selected_curve_nodes = curve
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| {
                    let display_x = CurvePreviewWidget::display_phase(node.x, phase_offset);
                    (survivors.contains(&index)
                        && display_x >= min_display_x
                        && display_x <= max_display_x
                        && node.y >= min_y
                        && node.y <= max_y)
                        .then_some(index)
                })
                .collect();
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::PressCurveOffset {
            pointer_x,
            quantized,
        } => {
            if !state
                .host_param_edit_sink
                .gesture_started(&state.automation_config, PARAM_PHASE_OFFSET_ID)
            {
                return;
            }
            state.clear_curve_selection();
            state.active_curve_offset = Some(ActiveCurveOffsetDrag {
                origin_phase_offset: state.params.phase_offset(),
                start_pointer_x: pointer_x,
                raw_delta: 0.0,
                quantized,
            });
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            state.command_hover_held = true;
            state.shift_hover_held = true;
        }
        CurvePreviewMessage::ResetCurveOffset => {
            if state.params.phase_offset().abs() <= f32::EPSILON
                || !state
                    .host_param_edit_sink
                    .gesture_started(&state.automation_config, PARAM_PHASE_OFFSET_ID)
            {
                return;
            }
            if state.host_param_edit_sink.gesture_value(
                &state.automation_config,
                PARAM_PHASE_OFFSET_ID,
                0.0,
            ) {
                state.params.set_phase_offset(0.0);
            }
            let _ = state
                .host_param_edit_sink
                .gesture_ended(&state.automation_config, PARAM_PHASE_OFFSET_ID);
        }
        CurvePreviewMessage::InsertNode { node, command_held } => {
            let mut curve = state.params.editable_curve_snapshot();
            state.clear_curve_selection();
            state.command_hover_held = command_held;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            if let Some(index) = insert_curve_node(&mut curve, node) {
                state.params.set_editable_curve(&curve);
                state.active_curve_node = Some(index);
                state.active_curve_node_drag =
                    start_curve_node_drag(&curve, index, node, false, false);
                state.hover_curve_node = Some(index);
            }
        }
        CurvePreviewMessage::DeleteNode { index } => {
            let mut curve = state.params.editable_curve_snapshot();
            if !interactive_curve_node_survivors(&curve, state.params.phase_offset(), None)
                .contains(&index)
            {
                return;
            }
            state.clear_curve_selection();
            if delete_curve_node(&mut curve, index) {
                state.params.set_editable_curve(&curve);
            }
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::DeleteSelectedNodes => {
            let mut curve = state.params.editable_curve_snapshot();
            let deleted = delete_selected_curve_nodes(
                &mut curve,
                &state.selected_curve_nodes,
                state.params.phase_offset(),
            );
            if deleted {
                state.params.set_editable_curve(&curve);
            }
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.active_curve_offset = None;
            state.active_curve_marquee = None;
            state.preview_curve_offset = None;
            state.selected_curve_nodes.clear();
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::DragNode {
            index,
            node,
            push_through_threshold_x,
        } => {
            let current_curve = state.params.editable_curve_snapshot();
            let current = state
                .active_curve_node
                .and_then(|active_index| current_curve.nodes.get(active_index))
                .copied()
                .unwrap_or(node);
            let shift_held = state.shift_hover_held;
            let option_held = state.option_hover_held;
            let command_held = state.command_hover_held;
            let (curve, moved_index) = if let Some(drag) = state.active_curve_node_drag.as_mut() {
                let target = drag.target_for_pointer(
                    node,
                    CurvePointDragModifiers {
                        shift_held,
                        option_held,
                        command_held,
                    },
                    state.params.sync_division(),
                    curve_width_from_push_through_threshold_x(push_through_threshold_x),
                    state.params.swing(),
                    current,
                );
                curve_with_dragged_node(drag, target, push_through_threshold_x)
            } else {
                let mut curve = current_curve;
                let mut target = node;
                if command_held {
                    target.x = snap_curve_time_to_beat_grid_with_swing(
                        state.params.sync_division(),
                        curve_width_from_push_through_threshold_x(push_through_threshold_x),
                        target.x,
                        state.params.swing(),
                    );
                }
                update_curve_node(&mut curve, index, target);
                (curve, index)
            };
            state.params.set_editable_curve(&curve);
            state.active_curve_node = Some(moved_index);
            state.active_curve_segment = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = Some(moved_index);
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::DragCurveOffset { delta } => {
            let Some(drag) = state.active_curve_offset.as_mut() else {
                return;
            };
            drag.raw_delta = delta;
            let phase_offset = resolve_curve_offset(
                state.params.sync_division(),
                (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0),
                state.params.swing(),
                drag.origin_phase_offset,
                delta,
                drag.quantized,
            );
            if state.host_param_edit_sink.gesture_value(
                &state.automation_config,
                PARAM_PHASE_OFFSET_ID,
                phase_offset as f64,
            ) {
                state.params.set_phase_offset(phase_offset);
            }
            state.active_curve_node = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::ReleaseCurveOffset { delta, option_held } => {
            if let Some(mut drag) = state.active_curve_offset.take() {
                drag.raw_delta = delta;
                drag.quantized = option_held;
                let phase_offset = resolve_curve_offset(
                    state.params.sync_division(),
                    (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0),
                    state.params.swing(),
                    drag.origin_phase_offset,
                    delta,
                    drag.quantized,
                );
                if state.host_param_edit_sink.gesture_value(
                    &state.automation_config,
                    PARAM_PHASE_OFFSET_ID,
                    phase_offset as f64,
                ) {
                    state.params.set_phase_offset(phase_offset);
                }
                let _ = state
                    .host_param_edit_sink
                    .gesture_ended(&state.automation_config, PARAM_PHASE_OFFSET_ID);
            }
            state.preview_curve_offset = None;
            state.option_hover_held = option_held;
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::ReleaseNode {
            index,
            node,
            push_through_threshold_x,
            shift_held,
            option_held,
            command_held,
        } => {
            let grouped_drag = state
                .active_curve_node_drag
                .as_ref()
                .is_some_and(|drag| !drag.selected_indices.is_empty());
            if !grouped_drag {
                state.clear_curve_selection();
            }
            let current_curve = state.params.editable_curve_snapshot();
            let current = state
                .active_curve_node
                .and_then(|active_index| current_curve.nodes.get(active_index))
                .copied()
                .unwrap_or(node);
            let (curve, moved_index) = if let Some(drag) = state.active_curve_node_drag.as_mut() {
                let target = drag.target_for_pointer(
                    node,
                    CurvePointDragModifiers {
                        shift_held,
                        option_held,
                        command_held,
                    },
                    state.params.sync_division(),
                    curve_width_from_push_through_threshold_x(push_through_threshold_x),
                    state.params.swing(),
                    current,
                );
                curve_with_dragged_node(drag, target, push_through_threshold_x)
            } else {
                let mut curve = current_curve;
                let mut target = node;
                if command_held {
                    target.x = snap_curve_time_to_beat_grid_with_swing(
                        state.params.sync_division(),
                        curve_width_from_push_through_threshold_x(push_through_threshold_x),
                        target.x,
                        state.params.swing(),
                    );
                }
                update_curve_node(&mut curve, index, target);
                (curve, index)
            };
            state.params.set_editable_curve(&curve);
            state.option_hover_held = option_held;
            state.command_hover_held = command_held;
            state.shift_hover_held = shift_held;
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = Some(moved_index);
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
        }
        CurvePreviewMessage::PressSegment { index, position } => {
            let curve = state.params.editable_curve_snapshot();
            if let Some(drag) = start_curve_segment_tension_drag(&curve, index, position) {
                state.active_curve_node = None;
                state.active_curve_node_drag = None;
                state.active_curve_paint = None;
                state.active_curve_offset = None;
                state.preview_curve_offset = None;
                state.active_curve_segment = Some(drag);
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(index);
            }
        }
        CurvePreviewMessage::PressSegmentMove { index, position } => {
            let curve = state.params.editable_curve_snapshot();
            if let Some(drag) = start_curve_segment_move_drag(&curve, index, position) {
                state.active_curve_node = None;
                state.active_curve_node_drag = None;
                state.active_curve_paint = None;
                state.active_curve_offset = None;
                state.preview_curve_offset = None;
                state.active_curve_segment = Some(drag);
                state.command_hover_held = true;
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(index);
            }
        }
        CurvePreviewMessage::DragSegment {
            index: _,
            position,
            curve_size,
        } => {
            if let Some(drag) = state.active_curve_segment.as_ref() {
                let curve = curve_with_dragged_segment(drag, position, curve_size);
                state.params.set_editable_curve(&curve);
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = Some(drag.index);
            }
        }
        CurvePreviewMessage::ReleaseSegment {
            index: _,
            position,
            curve_size,
        } => {
            if let Some(drag) = state.active_curve_segment.take() {
                let curve = curve_with_dragged_segment(&drag, position, curve_size);
                state.params.set_editable_curve(&curve);
                state.hover_curve_node = None;
                state.preview_curve_node = None;
                state.hover_curve_segment = match drag.mode {
                    CurveSegmentDragMode::MovePair => {
                        state.command_hover_held.then_some(drag.index)
                    }
                    CurveSegmentDragMode::AdjustTension { .. } => {
                        state.option_hover_held.then_some(drag.index)
                    }
                };
            }
        }
        CurvePreviewMessage::Cancel => {
            state.active_curve_paint = None;
            if let Some(drag) = state.active_curve_offset.take() {
                if state.params.phase_offset() != drag.origin_phase_offset
                    && state.host_param_edit_sink.gesture_value(
                        &state.automation_config,
                        PARAM_PHASE_OFFSET_ID,
                        drag.origin_phase_offset as f64,
                    )
                {
                    state.params.set_phase_offset(drag.origin_phase_offset);
                }
                let _ = state
                    .host_param_edit_sink
                    .gesture_ended(&state.automation_config, PARAM_PHASE_OFFSET_ID);
            }
            state.active_curve_node = None;
            state.active_curve_node_drag = None;
            state.active_curve_paint = None;
            state.active_curve_segment = None;
            state.active_curve_marquee = None;
            state.preview_curve_offset = None;
            state.hover_curve_node = None;
            state.preview_curve_node = None;
            state.hover_curve_segment = None;
            state.option_hover_held = false;
            state.command_hover_held = false;
            state.shift_hover_held = false;
        }
    }
}

fn commit_active_curve_paint(state: &mut RadiantEditorState, paint: ActiveCurvePaint) {
    match paint.finished_curve() {
        PaintCommitOutcome::Applied { candidate } => {
            state.params.set_editable_curve(&candidate);
            let mut origin_snapshot = paint.origin_snapshot;
            origin_snapshot.curve = paint.origin_curve;
            state.push_history_snapshot(origin_snapshot);
        }
        PaintCommitOutcome::NoOp { candidate: _ } => {}
    }
}

fn start_curve_node_drag(
    curve: &EditableCurve,
    index: usize,
    pointer: CurveNode,
    shift_held: bool,
    option_held: bool,
) -> Option<ActiveCurveNodeDrag> {
    let normalized = curve.clone().normalized();
    let origin = *normalized.nodes.get(index)?;
    let vertical_active = shift_held && option_held;
    let horizontal_active = shift_held && !vertical_active;
    Some(ActiveCurveNodeDrag {
        origin_index: index,
        origin_curve: normalized,
        selected_indices: Vec::new(),
        horizontal_gain_anchor: horizontal_active.then_some(origin.y),
        vertical_time_anchor: vertical_active.then_some(origin.x),
        last_pointer_x: pointer.x.clamp(0.0, 1.0),
        last_pointer_y: pointer.y.clamp(0.0, 1.0),
        unconstrained_x_offset: 0.0,
        unconstrained_y_offset: 0.0,
        suppress_command_snap_once: false,
    })
}

fn curve_with_dragged_node(
    drag: &ActiveCurveNodeDrag,
    target: CurveNode,
    push_through_threshold_x: f32,
) -> (EditableCurve, usize) {
    if !drag.selected_indices.is_empty() {
        return curve_with_dragged_selected_nodes(drag, target);
    }
    let mut curve = drag.origin_curve.clone();
    let moved_index = move_curve_node_with_push_through(
        &mut curve,
        drag.origin_index,
        target,
        push_through_threshold_x,
    );
    curve.normalize_in_place();
    (curve, moved_index)
}

/// Apply one pointer delta to all nodes in a retained marquee selection.
///
/// Unlike the single-node gesture, grouped movement never pushes through or
/// removes neighbours. The common x delta is clipped against every unselected
/// node (and the fixed endpoints), so the original ordering and minimum
/// spacing remain intact for the duration of the gesture.
fn curve_with_dragged_selected_nodes(
    drag: &ActiveCurveNodeDrag,
    target: CurveNode,
) -> (EditableCurve, usize) {
    let mut curve = drag.origin_curve.clone();
    let Some(anchor) = drag.origin_curve.nodes.get(drag.origin_index).copied() else {
        return (curve, drag.origin_index);
    };
    let selected: std::collections::HashSet<usize> = drag
        .selected_indices
        .iter()
        .copied()
        .filter(|index| *index < drag.origin_curve.nodes.len())
        .collect();
    if selected.is_empty() {
        return (curve, drag.origin_index);
    }

    let mut min_delta = -1.0_f32;
    let mut max_delta = 1.0_f32;
    for (index, node) in drag.origin_curve.nodes.iter().enumerate() {
        if !selected.contains(&index) || index == 0 || index + 1 == drag.origin_curve.nodes.len() {
            continue;
        }
        min_delta = min_delta.max(-node.x + CURVE_NODE_MIN_SPACING_X);
        max_delta = max_delta.min(1.0 - node.x - CURVE_NODE_MIN_SPACING_X);
        for (other_index, other) in drag.origin_curve.nodes.iter().enumerate() {
            // Endpoints are always fixed in x, even when marquee-selected.
            let other_selected = selected.contains(&other_index)
                && other_index > 0
                && other_index + 1 < drag.origin_curve.nodes.len();
            if other_selected {
                continue;
            }
            if other_index < index {
                min_delta = min_delta.max(other.x + CURVE_NODE_MIN_SPACING_X - node.x);
            } else if other_index > index {
                max_delta = max_delta.min(other.x - CURVE_NODE_MIN_SPACING_X - node.x);
            }
        }
    }
    let (min_delta, max_delta) = feasible_delta_interval_including_zero(min_delta, max_delta);
    let delta_x = (target.x - anchor.x).clamp(min_delta, max_delta);
    let delta_y = target.y - anchor.y;
    for index in selected.iter().copied() {
        let Some(origin) = drag.origin_curve.nodes.get(index).copied() else {
            continue;
        };
        let x = if index == 0 {
            0.0
        } else if index + 1 == drag.origin_curve.nodes.len() {
            1.0
        } else {
            (origin.x + delta_x).clamp(0.0, 1.0)
        };
        curve.nodes[index] = CurveNode {
            x,
            y: (origin.y + delta_y).clamp(0.0, 1.0),
        };
    }
    if selected.contains(&0) || selected.contains(&(curve.nodes.len().saturating_sub(1))) {
        let endpoint_y = selected
            .iter()
            .find_map(|index| {
                (*index == 0 || *index + 1 == curve.nodes.len()).then_some(curve.nodes[*index].y)
            })
            .unwrap_or(curve.nodes[0].y);
        set_wrapped_curve_endpoint_y(&mut curve, endpoint_y);
    } else {
        enforce_wrapped_curve_endpoints(&mut curve);
    }
    curve.normalize_in_place();
    (curve, drag.origin_index)
}

fn move_curve_node_with_push_through(
    curve: &mut EditableCurve,
    index: usize,
    target: CurveNode,
    push_through_threshold_x: f32,
) -> usize {
    if index >= curve.nodes.len() {
        return index;
    }

    let y = target.y.clamp(0.0, 1.0);
    let last_index = curve.nodes.len().saturating_sub(1);
    if index == 0 {
        set_wrapped_curve_endpoint_y(curve, y);
        return 0;
    }
    if index == last_index {
        set_wrapped_curve_endpoint_y(curve, y);
        return curve.nodes.len().saturating_sub(1);
    }

    let mut moved_index = index;
    let threshold_x = push_through_threshold_x.max(0.0);
    while moved_index + 1 < curve.nodes.len().saturating_sub(1)
        && target.x > curve.nodes[moved_index + 1].x + threshold_x
    {
        remove_interior_curve_node(curve, moved_index + 1);
    }
    while moved_index > 1 && target.x < curve.nodes[moved_index - 1].x - threshold_x {
        remove_interior_curve_node(curve, moved_index - 1);
        moved_index = moved_index.saturating_sub(1);
    }

    let origin_x = curve.nodes[moved_index].x;
    let preferred_min_x = curve.nodes[moved_index - 1].x + CURVE_NODE_MIN_SPACING_X;
    let preferred_max_x = curve.nodes[moved_index + 1].x - CURVE_NODE_MIN_SPACING_X;
    let (min_x, max_x) =
        feasible_node_x_interval_including_origin(preferred_min_x, preferred_max_x, origin_x);
    curve.nodes[moved_index] = CurveNode {
        x: target.x.clamp(min_x, max_x),
        y,
    };
    enforce_wrapped_curve_endpoints(curve);
    moved_index
}

fn remove_interior_curve_node(curve: &mut EditableCurve, remove_index: usize) {
    let last_index = curve.nodes.len().saturating_sub(1);
    if remove_index == 0 || remove_index >= last_index {
        return;
    }

    let left_segment_index = remove_index.saturating_sub(1);
    let right_segment_index = remove_index.min(curve.segments.len().saturating_sub(1));
    let left_tension = curve
        .segments
        .get(left_segment_index)
        .copied()
        .unwrap_or(CurveSegment { tension: 0.0 })
        .tension;
    let right_tension = curve
        .segments
        .get(right_segment_index)
        .copied()
        .unwrap_or(CurveSegment {
            tension: left_tension,
        })
        .tension;
    let merged_tension =
        ((left_tension + right_tension) * 0.5).clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION);

    curve.nodes.remove(remove_index);
    if !curve.segments.is_empty() {
        if right_segment_index < curve.segments.len() {
            curve.segments.remove(right_segment_index);
        } else {
            curve.segments.pop();
        }
        if left_segment_index < curve.segments.len() {
            curve.segments[left_segment_index].tension = merged_tension;
        }
    }
}

fn set_wrapped_curve_endpoint_y(curve: &mut EditableCurve, y: f32) {
    if curve.nodes.len() < 2 {
        return;
    }
    let clamped = y.clamp(0.0, 1.0);
    let last_index = curve.nodes.len() - 1;
    curve.nodes[0] = CurveNode { x: 0.0, y: clamped };
    curve.nodes[last_index] = CurveNode { x: 1.0, y: clamped };
}

fn enforce_wrapped_curve_endpoints(curve: &mut EditableCurve) {
    if curve.nodes.len() < 2 {
        return;
    }
    set_wrapped_curve_endpoint_y(curve, curve.nodes[0].y);
}

fn start_curve_segment_tension_drag(
    curve: &EditableCurve,
    index: usize,
    start_pointer: Point,
) -> Option<ActiveCurveSegmentDrag> {
    let normalized = curve.clone().normalized();
    let start_tension = normalized.segments.get(index).copied()?.tension;
    (index + 1 < normalized.nodes.len()).then_some(ActiveCurveSegmentDrag {
        index,
        origin_curve: normalized,
        start_pointer,
        mode: CurveSegmentDragMode::AdjustTension { start_tension },
    })
}

fn start_curve_segment_move_drag(
    curve: &EditableCurve,
    index: usize,
    start_pointer: Point,
) -> Option<ActiveCurveSegmentDrag> {
    let normalized = curve.clone().normalized();
    (index + 1 < normalized.nodes.len()).then_some(ActiveCurveSegmentDrag {
        index,
        origin_curve: normalized,
        start_pointer,
        mode: CurveSegmentDragMode::MovePair,
    })
}

fn curve_with_dragged_segment(
    drag: &ActiveCurveSegmentDrag,
    current_pointer: Point,
    curve_size: Vector2,
) -> EditableCurve {
    let mut curve = drag.origin_curve.clone();
    match drag.mode {
        CurveSegmentDragMode::AdjustTension { start_tension } => {
            let delta = segment_tension_delta_from_drag(
                &drag.origin_curve,
                drag.index,
                drag.start_pointer,
                current_pointer,
            );
            if let Some(segment) = curve.segments.get_mut(drag.index) {
                segment.tension =
                    (start_tension + delta).clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION);
            }
        }
        CurveSegmentDragMode::MovePair => {
            let curve_width = curve_size.x.max(2.0);
            let curve_height = curve_size.y.max(2.0);
            let delta = (
                (current_pointer.x - drag.start_pointer.x) / (curve_width - 1.0),
                (drag.start_pointer.y - current_pointer.y) / (curve_height - 1.0),
            );
            let left = drag.origin_curve.nodes[drag.index];
            let right = drag.origin_curve.nodes[drag.index + 1];
            super::move_segment_translated(
                &mut curve,
                drag.index,
                (left.x, left.y),
                (right.x, right.y),
                delta,
            );
        }
    }
    curve.normalize_in_place();
    curve
}

fn segment_tension_delta_from_drag(
    curve: &EditableCurve,
    segment_index: usize,
    start_pointer: Point,
    current_pointer: Point,
) -> f32 {
    let drag_units = (start_pointer.y - current_pointer.y) / CURVE_SEGMENT_TENSION_PIXEL_SCALE;
    drag_units * segment_upward_tension_sign(curve, segment_index)
}

fn segment_upward_tension_sign(curve: &EditableCurve, segment_index: usize) -> f32 {
    let left = curve.nodes.get(segment_index).copied();
    let right = curve.nodes.get(segment_index + 1).copied();
    match (left, right) {
        (Some(left_node), Some(right_node)) if right_node.y > left_node.y => -1.0,
        _ => 1.0,
    }
}

fn insert_curve_node(curve: &mut EditableCurve, node: CurveNode) -> Option<usize> {
    curve.normalize_in_place();
    if curve.nodes.len() < 2 || curve.nodes.len() >= MAX_EDITABLE_NODES {
        return None;
    }

    let mut insert_at = curve.nodes.partition_point(|existing| existing.x < node.x);
    insert_at = insert_at.clamp(1, curve.nodes.len().saturating_sub(1));

    let left_limit = curve.nodes[insert_at - 1].x + CURVE_NODE_MIN_SPACING_X;
    let right_limit = curve.nodes[insert_at].x - CURVE_NODE_MIN_SPACING_X;
    if left_limit >= right_limit {
        return None;
    }

    curve.nodes.insert(
        insert_at,
        CurveNode {
            x: node.x.clamp(left_limit, right_limit),
            y: node.y.clamp(0.0, 1.0),
        },
    );
    let inherited = curve
        .segments
        .get(insert_at.saturating_sub(1))
        .copied()
        .unwrap_or(CurveSegment { tension: 0.0 });
    curve
        .segments
        .insert(insert_at.saturating_sub(1), inherited);
    curve.normalize_in_place();
    Some(insert_at)
}

fn delete_curve_node(curve: &mut EditableCurve, index: usize) -> bool {
    curve.normalize_in_place();
    if index == 0 || index + 1 >= curve.nodes.len() {
        return false;
    }

    curve.nodes.remove(index);
    if !curve.segments.is_empty() {
        let remove_segment = index
            .saturating_sub(1)
            .min(curve.segments.len().saturating_sub(1));
        curve.segments.remove(remove_segment);
    }
    curve.normalize_in_place();
    true
}

fn delete_selected_curve_nodes(
    curve: &mut EditableCurve,
    selected: &[usize],
    phase_offset: f32,
) -> bool {
    let survivors = interactive_curve_node_survivors(curve, phase_offset, None);
    let mut indices: Vec<usize> = selected
        .iter()
        .copied()
        .filter(|index| survivors.contains(index) && *index > 0 && *index + 1 < curve.nodes.len())
        .collect();
    indices.sort_unstable();
    indices.dedup();
    let mut deleted = false;
    for index in indices.into_iter().rev() {
        deleted |= delete_curve_node(curve, index);
    }
    deleted
}

fn update_curve_node(curve: &mut EditableCurve, index: usize, node: CurveNode) {
    if curve.nodes.is_empty() || index >= curve.nodes.len() {
        return;
    }
    if index == 0 || index + 1 == curve.nodes.len() {
        let y = node.y.clamp(0.0, 1.0);
        if let Some(first) = curve.nodes.first_mut() {
            first.x = 0.0;
            first.y = y;
        }
        if let Some(last) = curve.nodes.last_mut() {
            last.x = 1.0;
            last.y = y;
        }
        curve.normalize_in_place();
        return;
    }

    let previous_x = curve.nodes[index - 1].x + CURVE_NODE_MIN_SPACING_X;
    let next_x = curve.nodes[index + 1].x - CURVE_NODE_MIN_SPACING_X;
    if previous_x >= next_x {
        return;
    }
    curve.nodes[index] = CurveNode {
        x: node.x.clamp(previous_x, next_x),
        y: node.y.clamp(0.0, 1.0),
    };
    curve.normalize_in_place();
}

fn normalize_output_gain(value: f32) -> f32 {
    ((value - MIN_OUTPUT_GAIN_DB) / (MAX_OUTPUT_GAIN_DB - MIN_OUTPUT_GAIN_DB)).clamp(0.0, 1.0)
}

fn denormalize_output_gain(value: f32) -> f32 {
    MIN_OUTPUT_GAIN_DB + value.clamp(0.0, 1.0) * (MAX_OUTPUT_GAIN_DB - MIN_OUTPUT_GAIN_DB)
}

fn normalize_sync_division(value: usize) -> f32 {
    (value as f32 / MAX_SYNC_DIVISION).clamp(0.0, 1.0)
}

#[derive(Clone)]
struct NumericValueLabelWidget {
    common: WidgetCommon,
    target: NumericEntryTarget,
    text: String,
    editing: bool,
    dirty: bool,
}

impl NumericValueLabelWidget {
    fn new(target: NumericEntryTarget, text: String, editing: bool, dirty: bool) -> Self {
        Self {
            common: WidgetCommon::fixed(0, CONTROL_VALUE_WIDTH, CONTROL_ROW_HEIGHT)
                .with_focus(FocusBehavior::Keyboard)
                .without_default_chrome(),
            target,
            text,
            editing,
            dirty,
        }
    }

    fn draft_with_character(&self, ch: char) -> Option<String> {
        if !is_numeric_entry_char(ch) {
            return None;
        }
        let mut draft = if self.dirty {
            self.text.clone()
        } else {
            String::new()
        };
        if draft.chars().count() >= VALUE_ENTRY_MAX_CHARS {
            return None;
        }
        draft.push(ch);
        Some(draft)
    }

    fn draft_after_backspace(&self) -> String {
        if !self.dirty {
            return String::new();
        }
        let mut draft = self.text.clone();
        draft.pop();
        draft
    }

    fn draft_after_delete(&self) -> String {
        if self.dirty {
            self.text.clone()
        } else {
            String::new()
        }
    }

    fn changed(&self, draft: String) -> NumericEntryMessage {
        NumericEntryMessage::DraftChanged {
            target: self.target,
            draft,
            dirty: true,
        }
    }
}

impl Widget for NumericValueLabelWidget {
    fn common(&self) -> &WidgetCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        &mut self.common
    }

    fn handle_input(&mut self, bounds: Rect, input: WidgetInput) -> Option<WidgetOutput> {
        let message = match input {
            WidgetInput::PointerMove { position } => {
                self.common.state.hovered = bounds.contains(position);
                None
            }
            WidgetInput::PointerPress {
                position,
                button: PointerButton::Primary,
                modifiers,
            } if bounds.contains(position) => {
                self.common.state.focused = true;
                self.common.state.hovered = true;
                modifiers.command.then_some(NumericEntryMessage::Begin {
                    target: self.target,
                })
            }
            WidgetInput::FocusChanged(focused) => {
                self.common.state.focused = focused;
                (!focused && self.editing).then_some(NumericEntryMessage::Cancel {
                    target: self.target,
                })
            }
            WidgetInput::Character('\u{1b}') if self.editing => Some(NumericEntryMessage::Cancel {
                target: self.target,
            }),
            WidgetInput::Character('\r' | '\n') if self.editing => {
                Some(NumericEntryMessage::Commit {
                    target: self.target,
                    draft: self.text.clone(),
                })
            }
            WidgetInput::Character(ch) if self.editing => self
                .draft_with_character(ch)
                .map(|draft| self.changed(draft)),
            WidgetInput::KeyPress(WidgetKey::Enter) if self.editing => {
                Some(NumericEntryMessage::Commit {
                    target: self.target,
                    draft: self.text.clone(),
                })
            }
            WidgetInput::KeyPress(WidgetKey::Backspace) if self.editing => {
                Some(self.changed(self.draft_after_backspace()))
            }
            WidgetInput::KeyPress(WidgetKey::Delete) if self.editing => {
                Some(self.changed(self.draft_after_delete()))
            }
            _ => None,
        }?;
        Some(WidgetOutput::typed(message))
    }

    fn accepts_text_input(&self) -> bool {
        self.editing
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        if self.editing {
            primitives.push(PaintPrimitive::FillRect(PaintFillRect {
                widget_id: self.common.id,
                rect: bounds,
                color: theme.bg_primary,
            }));
            primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
                widget_id: self.common.id,
                rect: bounds,
                color: if self.common.state.focused {
                    theme.accent_warning
                } else {
                    theme.border_emphasis
                },
                width: 1.0,
            }));
        }
        primitives.push(PaintPrimitive::Text(PaintTextRun {
            widget_id: self.common.id,
            text: PaintText::from(self.text.clone()),
            rect: bounds,
            font_size: VALUE_LABEL_FONT_SIZE,
            baseline: None,
            color: if self.editing {
                theme.text_primary
            } else {
                theme.text_muted
            },
            align: if self.editing {
                PaintTextAlign::Left
            } else {
                PaintTextAlign::Center
            },
            wrap: TextWrap::None,
        }));
    }
}

impl WidgetSemantics for NumericValueLabelWidget {
    fn automation_label(&self) -> Option<String> {
        Some(format!("{} value", self.target.label()))
    }

    fn automation_value_text(&self) -> Option<String> {
        Some(self.text.clone())
    }
}

fn is_numeric_entry_char(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+' | '%' | ' ' | 'd' | 'D' | 'b' | 'B')
}

#[derive(Clone)]
struct CurveSlotWidget {
    common: WidgetCommon,
    index: usize,
    curve: Option<EditableCurve>,
    loaded: bool,
    deviated: bool,
    command_hovered: bool,
}

impl CurveSlotWidget {
    fn new(index: usize, curve: Option<EditableCurve>, loaded: bool, deviated: bool) -> Self {
        Self {
            // The row assigns each slot an equal fluid width.  A fixed width
            // here would consume the compact inner width and clip the eighth slot.
            common: WidgetCommon::fixed(0, 1.0, CURVE_SLOT_ROW_HEIGHT)
                .with_keyboard_focus()
                .without_default_chrome(),
            index,
            curve: curve.map(|curve| curve.normalized()),
            loaded,
            deviated,
            command_hovered: false,
        }
    }

    fn sample_points(&self, bounds: Rect) -> Arc<[Point]> {
        let preview = bounds.inset(
            (CURVE_SLOT_MARGIN + 4.25)
                .min(bounds.width() * 0.25)
                .max(1.0),
            6.8_f32.min(bounds.height() * 0.25).max(1.0),
            (CURVE_SLOT_MARGIN + 4.25)
                .min(bounds.width() * 0.25)
                .max(1.0),
            6.8_f32.min(bounds.height() * 0.25).max(1.0),
        );
        let inner_w = preview.width().max(1.0);
        let inner_h = preview.height().max(1.0);
        if let Some(curve) = self.curve.as_ref() {
            let points: Vec<Point> = (0..CURVE_SLOT_PREVIEW_STEPS.max(2))
                .map(|step| {
                    let steps = CURVE_SLOT_PREVIEW_STEPS.max(2);
                    let t = step as f32 / (steps - 1) as f32;
                    Point::new(
                        preview.min.x + t * inner_w,
                        preview.min.y
                            + (1.0 - sample_editable_curve(curve, t).clamp(0.0, 1.0)) * inner_h,
                    )
                })
                .collect();
            Arc::from(points)
        } else {
            let y = preview.min.y + inner_h * 0.5;
            Arc::from([Point::new(preview.min.x, y), Point::new(preview.max.x, y)])
        }
    }
}

impl Widget for CurveSlotWidget {
    fn common(&self) -> &WidgetCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        &mut self.common
    }

    fn handle_input(&mut self, bounds: Rect, input: WidgetInput) -> Option<WidgetOutput> {
        let message = match input {
            WidgetInput::PointerMove { position } => {
                self.common.state.hovered = bounds.contains(position);
                None
            }
            WidgetInput::PointerModifiersChanged { modifiers } => {
                self.command_hovered = modifiers.command;
                None
            }
            WidgetInput::PointerPress {
                position,
                button: PointerButton::Primary,
                modifiers,
            } if bounds.contains(position) => {
                self.common.state.focused = true;
                self.common.state.hovered = true;
                self.common.state.pressed = true;
                self.command_hovered = modifiers.command;
                if modifiers.command {
                    Some(CurveSlotMessage::Store { index: self.index })
                } else {
                    Some(CurveSlotMessage::Load { index: self.index })
                }
            }
            WidgetInput::PointerRelease { .. } | WidgetInput::PointerDrop { .. } => {
                self.common.state.pressed = false;
                None
            }
            WidgetInput::FocusChanged(focused) => {
                self.common.state.focused = focused;
                None
            }
            _ => None,
        }?;
        Some(WidgetOutput::typed(message))
    }

    fn synchronize_from_previous(&mut self, previous: &dyn Widget) {
        let Some(previous) = previous.as_any().downcast_ref::<Self>() else {
            return;
        };
        self.common.state.hovered = previous.common.state.hovered;
        self.common.state.pressed = previous.common.state.pressed;
        self.common.state.focused = previous.common.state.focused;
        self.command_hovered = previous.command_hovered;
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        let hovered = self.common.state.hovered;
        let pressed = self.common.state.pressed;
        let fill = if self.deviated {
            theme.accent_danger
        } else if pressed {
            theme.accent_copper.with_alpha(96)
        } else if self.loaded {
            theme.surface_raised
        } else if hovered {
            theme.surface_raised.with_alpha(220)
        } else {
            theme.surface_raised.with_alpha(176)
        };
        let curve_color = if self.deviated || self.curve.is_some() {
            theme.accent_copper
        } else {
            theme.text_muted
        };
        let card_rect = bounds.inset(1.0, 1.0, 1.0, 1.0);
        primitives.push(PaintPrimitive::FillPath(PaintFillPath::new(
            self.common.id,
            PaintPath::from(rounded_rect_commands(card_rect, 5.95)),
            PaintBrush::solid(fill),
        )));
        let border = if self.deviated {
            theme.accent_danger
        } else if self.loaded || hovered || pressed {
            theme.accent_copper
        } else {
            theme.border.with_alpha(190)
        };
        if self.loaded {
            primitives.push(PaintPrimitive::FillPath(
                PaintFillPath::new(
                    self.common.id,
                    rounded_ring_path(bounds.inset(0.25, 0.25, 0.25, 0.25), 6.8, 2.975),
                    PaintBrush::solid(theme.accent_copper.with_alpha(64)),
                )
                .fill_rule(PaintFillRule::EvenOdd),
            ));
        }
        primitives.push(PaintPrimitive::FillPath(
            PaintFillPath::new(
                self.common.id,
                rounded_ring_path(bounds.inset(0.75, 0.75, 0.75, 0.75), 6.375, 1.0),
                PaintBrush::solid(border),
            )
            .fill_rule(PaintFillRule::EvenOdd),
        ));
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: self.sample_points(bounds),
            color: curve_color,
            width: if hovered || pressed || self.loaded || self.deviated {
                1.9125
            } else {
                1.4875
            },
        }));
        if self.command_hovered {
            let center = Point::new(bounds.max.x - 4.25, bounds.min.y + 4.25);
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.common.id,
                points: Arc::from([
                    Point::new(center.x - 1.7, center.y),
                    Point::new(center.x + 1.7, center.y),
                ]),
                color: theme.accent_copper,
                width: 1.0,
            }));
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.common.id,
                points: Arc::from([
                    Point::new(center.x, center.y - 1.7),
                    Point::new(center.x, center.y + 1.7),
                ]),
                color: theme.accent_copper,
                width: 1.0,
            }));
        }
    }
}

impl WidgetSemantics for CurveSlotWidget {
    fn automation_label(&self) -> Option<String> {
        Some(format!("Curve slot {}", self.index + 1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CurveSlotMessage {
    Load { index: usize },
    Store { index: usize },
}

#[derive(Clone)]
struct ParameterDeckDividerWidget {
    common: WidgetCommon,
}

impl ParameterDeckDividerWidget {
    fn new() -> Self {
        Self {
            common: WidgetCommon::fixed(0, PUMP_VISUAL_METRICS.divider, PARAMETER_DECK_HEIGHT)
                .without_default_chrome(),
        }
    }
}

impl Widget for ParameterDeckDividerWidget {
    fn common(&self) -> &WidgetCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        &mut self.common
    }

    fn handle_input(&mut self, _bounds: Rect, _input: WidgetInput) -> Option<WidgetOutput> {
        None
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        let x = bounds.min.x + bounds.width() * 0.5;
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: Arc::from([
                Point::new(x, bounds.min.y + PUMP_VISUAL_METRICS.space_4),
                Point::new(x, bounds.max.y - PUMP_VISUAL_METRICS.space_4),
            ]),
            color: theme.grid_strong,
            width: PUMP_VISUAL_METRICS.divider,
        }));
    }
}

#[derive(Clone)]
struct GainReductionMeterWidget {
    common: WidgetCommon,
    reduction_db: f32,
}

impl GainReductionMeterWidget {
    fn new(reduction_db: f32) -> Self {
        Self {
            common: WidgetCommon::fixed(0, GAIN_REDUCTION_METER_WIDTH, CURVE_PREVIEW_HEIGHT)
                .without_default_chrome(),
            reduction_db: reduction_db.clamp(0.0, crate::gui_status::GAIN_REDUCTION_METER_MAX_DB),
        }
    }
}

impl Widget for GainReductionMeterWidget {
    fn common(&self) -> &WidgetCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        &mut self.common
    }

    fn handle_input(&mut self, _bounds: Rect, _input: WidgetInput) -> Option<WidgetOutput> {
        None
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        _theme: &ThemeTokens,
    ) {
        let meter_colors = pump_meter_colors();
        let title_height = PUMP_TYPOGRAPHY.meta.1;
        let value_height = PUMP_TYPOGRAPHY.meta.1;
        let bar_top = bounds.min.y + title_height;
        let bar_height = (bounds.height() - title_height - value_height).max(1.0);
        let bar_left = bounds.min.x + (bounds.width() - GAIN_REDUCTION_METER_BAR_WIDTH) * 0.5;
        let bar = Rect::from_xy_size(
            bar_left,
            bar_top,
            GAIN_REDUCTION_METER_BAR_WIDTH,
            bar_height,
        );
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.common.id,
            rect: bar,
            color: meter_colors.track,
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.common.id,
            rect: bar,
            color: meter_colors.border,
            width: PUMP_VISUAL_METRICS.border,
        }));
        let fraction = crate::gui_status::gain_reduction_meter_fraction(self.reduction_db);
        let segment_step =
            PUMP_VISUAL_METRICS.meter_segment + PUMP_VISUAL_METRICS.meter_segment_gap;
        let segment_count = ((bar.height() - 2.0 + PUMP_VISUAL_METRICS.meter_segment_gap)
            / segment_step)
            .floor()
            .max(1.0) as usize;
        let active_segments = (fraction * segment_count as f32).round() as usize;
        for segment in 0..segment_count {
            let y = bar.min.y + 1.0 + segment as f32 * segment_step;
            primitives.push(PaintPrimitive::FillRect(PaintFillRect {
                widget_id: self.common.id,
                rect: Rect::from_xy_size(
                    bar.min.x + 1.0,
                    y,
                    (bar.width() - 2.0).max(0.0),
                    PUMP_VISUAL_METRICS.meter_segment,
                ),
                color: if segment < active_segments {
                    if fraction >= 0.75 {
                        meter_colors.hot
                    } else {
                        meter_colors.nominal
                    }
                } else {
                    meter_colors.track
                },
            }));
        }
        for (text_value, rect) in [
            (
                "GR dB".to_string(),
                Rect::from_xy_size(bounds.min.x, bounds.min.y, bounds.width(), title_height),
            ),
            (
                format!("{:.1}", self.reduction_db),
                Rect::from_xy_size(
                    bounds.min.x,
                    bounds.max.y - value_height,
                    bounds.width(),
                    value_height,
                ),
            ),
        ] {
            primitives.push(PaintPrimitive::Text(PaintTextRun {
                widget_id: self.common.id,
                text: PaintText::from(text_value),
                rect,
                font_size: PUMP_TYPOGRAPHY.meta.0,
                baseline: None,
                color: meter_colors.text,
                align: PaintTextAlign::Center,
                wrap: TextWrap::None,
            }));
        }
    }
}

#[derive(Clone)]
struct CurvePreviewWidget {
    common: WidgetCommon,
    curve: EditableCurve,
    active_node: Option<usize>,
    active_segment: Option<usize>,
    hover_node: Option<usize>,
    preview_node: Option<CurveNode>,
    hover_segment: Option<usize>,
    option_hover_held: bool,
    command_hover_held: bool,
    shift_hover_held: bool,
    active_segment_move: bool,
    active_curve_paint: bool,
    curve_paint_runs: Vec<PaintRun>,
    active_curve_offset_start_x: Option<f32>,
    selected_nodes: Vec<usize>,
    active_marquee: Option<ActiveCurveMarquee>,
    playhead_phase: Option<f32>,
    incoming_waveform: Option<IncomingWaveformSnapshot>,
    sync_division: usize,
    swing: f32,
    smooth: f32,
    phase_offset: f32,
    depth_db: f32,
    floor_db: f32,
}

impl CurvePreviewWidget {
    fn new(
        curve: EditableCurve,
        active_node: Option<usize>,
        active_segment: Option<usize>,
        hover_node: Option<usize>,
        preview_node: Option<CurveNode>,
        hover_segment: Option<usize>,
        option_hover_held: bool,
    ) -> Self {
        Self {
            common: WidgetCommon::fixed(
                0,
                (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0),
                CURVE_PREVIEW_HEIGHT,
            )
            .with_keyboard_focus()
            .without_default_chrome(),
            curve: curve.normalized(),
            active_node,
            active_segment,
            hover_node,
            preview_node,
            hover_segment,
            option_hover_held,
            command_hover_held: false,
            shift_hover_held: false,
            active_segment_move: false,
            active_curve_paint: false,
            curve_paint_runs: Vec::new(),
            active_curve_offset_start_x: None,
            selected_nodes: Vec::new(),
            active_marquee: None,
            playhead_phase: None,
            incoming_waveform: None,
            sync_division: crate::params::DEFAULT_SYNC_DIVISION_INDEX,
            swing: crate::params::DEFAULT_SWING,
            smooth: crate::params::DEFAULT_SMOOTH,
            phase_offset: crate::params::DEFAULT_PHASE_OFFSET,
            depth_db: crate::params::DEFAULT_DEPTH_DB,
            floor_db: crate::params::DEFAULT_FLOOR_DB,
        }
    }

    fn with_playhead_phase(mut self, playhead_phase: Option<f32>) -> Self {
        self.playhead_phase = playhead_phase.map(|phase| phase.rem_euclid(1.0));
        self
    }

    fn with_command_hover_held(mut self, command_hover_held: bool) -> Self {
        self.command_hover_held = command_hover_held;
        self
    }

    fn with_shift_hover_held(mut self, shift_hover_held: bool) -> Self {
        self.shift_hover_held = shift_hover_held;
        self
    }

    fn with_active_segment_move(mut self, active_segment_move: bool) -> Self {
        self.active_segment_move = active_segment_move;
        self
    }

    fn with_active_curve_paint(mut self, active_curve_paint: bool) -> Self {
        self.active_curve_paint = active_curve_paint;
        self
    }

    fn with_curve_paint_runs(mut self, runs: Option<Vec<PaintRun>>) -> Self {
        self.curve_paint_runs = runs.unwrap_or_default();
        self
    }

    fn with_active_curve_offset(mut self, start_pointer_x: Option<f32>) -> Self {
        self.active_curve_offset_start_x = start_pointer_x;
        self
    }

    fn with_selected_curve_nodes(mut self, selected_nodes: &[usize]) -> Self {
        self.selected_nodes = selected_nodes.to_vec();
        self
    }

    fn with_active_curve_marquee(mut self, marquee: Option<ActiveCurveMarquee>) -> Self {
        self.active_marquee = marquee;
        self
    }

    fn with_incoming_waveform(
        mut self,
        incoming_waveform: Option<IncomingWaveformSnapshot>,
    ) -> Self {
        self.incoming_waveform = incoming_waveform;
        self
    }

    fn with_sync_division(mut self, sync_division: usize) -> Self {
        self.sync_division = sync_division;
        self
    }

    fn with_swing(mut self, swing: f32) -> Self {
        self.swing = swing.clamp(0.0, 1.0);
        self
    }

    fn with_smooth(mut self, smooth: f32) -> Self {
        self.smooth = smooth.clamp(0.0, 1.0);
        self
    }

    fn with_phase_offset(mut self, phase_offset: f32) -> Self {
        self.phase_offset = phase_offset.rem_euclid(1.0);
        self
    }

    fn with_gain_mapping(mut self, depth_db: f32, floor_db: f32) -> Self {
        self.depth_db = depth_db;
        self.floor_db = floor_db;
        self
    }

    fn curve_bounds(bounds: Rect) -> Rect {
        let gutter_width = curve_reference_gutter_width(bounds.width());
        Rect::from_xy_size(
            bounds.min.x + gutter_width,
            bounds.min.y,
            curve_viewport_width(bounds.width()),
            (bounds.height() - CURVE_OFFSET_BAR_HEIGHT - CURVE_OFFSET_BAR_INSET).max(1.0),
        )
    }

    fn offset_bar_bounds(bounds: Rect) -> Rect {
        let curve_bounds = Self::curve_bounds(bounds);
        Rect::from_xy_size(
            curve_bounds.min.x,
            curve_bounds.max.y + CURVE_OFFSET_BAR_INSET,
            curve_bounds.width(),
            CURVE_OFFSET_BAR_HEIGHT,
        )
    }

    fn reference_gutter_bounds(bounds: Rect) -> Rect {
        let curve_bounds = Self::curve_bounds(bounds);
        Rect::from_xy_size(
            bounds.min.x,
            bounds.min.y,
            (curve_bounds.min.x - bounds.min.x).max(0.0),
            bounds.height(),
        )
    }

    fn curve_point(bounds: Rect, node: CurveNode) -> Point {
        let curve_bounds = Self::curve_bounds(bounds);
        let width = curve_bounds.width().max(1.0) - 1.0;
        let height = curve_bounds.height().max(1.0) - 1.0;
        Point::new(
            curve_bounds.min.x + node.x.clamp(0.0, 1.0) * width,
            curve_bounds.min.y + (1.0 - node.y.clamp(0.0, 1.0)) * height,
        )
    }

    fn node_from_point(bounds: Rect, position: Point) -> CurveNode {
        let curve_bounds = Self::curve_bounds(bounds);
        let width = (curve_bounds.width().max(1.0) - 1.0).max(1.0);
        let height = (curve_bounds.height().max(1.0) - 1.0).max(1.0);
        CurveNode {
            x: ((position.x - curve_bounds.min.x) / width).clamp(0.0, 1.0),
            y: (1.0 - ((position.y - curve_bounds.min.y) / height)).clamp(0.0, 1.0),
        }
    }

    fn raw_node_from_display_point(&self, bounds: Rect, position: Point) -> CurveNode {
        let display_node = Self::node_from_point(bounds, position);
        let mut raw_x = (display_node.x - self.phase_offset).rem_euclid(1.0);
        let at_offset_seam = self.phase_offset > CURVE_DISPLAY_SEAM_EPSILON
            && (display_node.x - self.phase_offset).abs() <= CURVE_DISPLAY_SEAM_EPSILON;
        if at_offset_seam && self.active_node.is_some() {
            let current_display_x = self
                .active_node
                .and_then(|index| self.curve.nodes.get(index).copied())
                .map(|node| Self::display_phase(node.x, self.phase_offset))
                .unwrap_or(self.phase_offset);
            raw_x = if current_display_x > self.phase_offset {
                1.0 - CURVE_DISPLAY_SEAM_EPSILON
            } else {
                CURVE_DISPLAY_SEAM_EPSILON
            };
        } else if display_node.x <= CURVE_DISPLAY_SEAM_EPSILON
            && self.phase_offset.abs() > CURVE_DISPLAY_SEAM_EPSILON
        {
            raw_x = (raw_x + CURVE_DISPLAY_SEAM_EPSILON).rem_euclid(1.0);
        } else if display_node.x >= 1.0 - CURVE_DISPLAY_SEAM_EPSILON {
            raw_x = (raw_x - CURVE_DISPLAY_SEAM_EPSILON).rem_euclid(1.0);
        }
        CurveNode {
            x: raw_x,
            y: display_node.y,
        }
    }

    fn paint_sample_from_display_point(&self, bounds: Rect, position: Point) -> CurvePaintSample {
        CurvePaintSample {
            node: self.raw_node_from_display_point(bounds, position),
            display_position: Self::normalized_display_position(bounds, position),
            outside: false,
        }
    }

    fn paint_sample_from_boundary(&self, bounds: Rect, position: Point) -> CurvePaintSample {
        let curve_bounds = Self::curve_bounds(bounds);
        let display_position = Self::normalized_display_position(bounds, position);
        let projected = Point::new(
            if position.x.is_finite() {
                position.x.clamp(curve_bounds.min.x, curve_bounds.max.x)
            } else {
                curve_bounds.center().x
            },
            if position.y.is_finite() {
                position.y.clamp(curve_bounds.min.y, curve_bounds.max.y)
            } else {
                curve_bounds.center().y
            },
        );
        let mut sample = self.paint_sample_from_display_point(bounds, projected);
        sample.display_position = display_position;
        sample.outside = true;
        sample
    }

    fn normalized_display_position(bounds: Rect, position: Point) -> RectPoint {
        let curve_bounds = Self::curve_bounds(bounds);
        let width = (curve_bounds.width().max(1.0) - 1.0).max(1.0);
        let height = (curve_bounds.height().max(1.0) - 1.0).max(1.0);
        RectPoint {
            x: if position.x.is_finite() {
                (position.x - curve_bounds.min.x) / width
            } else {
                0.5
            },
            y: if position.y.is_finite() {
                1.0 - (position.y - curve_bounds.min.y) / height
            } else {
                0.5
            },
        }
    }

    fn display_phase(raw_x: f32, phase_offset: f32) -> f32 {
        let shifted = raw_x + phase_offset;
        let wrapped = shifted.rem_euclid(1.0);
        if shifted > CURVE_DISPLAY_SEAM_EPSILON && wrapped <= CURVE_DISPLAY_SEAM_EPSILON {
            1.0
        } else {
            wrapped
        }
    }

    fn display_node(&self, node: CurveNode) -> CurveNode {
        CurveNode {
            x: Self::display_phase(node.x, self.phase_offset),
            y: node.y,
        }
    }

    fn interactive_node_survivors(&self) -> Vec<usize> {
        interactive_curve_node_survivors(&self.curve, self.phase_offset, self.active_node)
    }

    fn has_interactive_deletable_selection(&self) -> bool {
        let survivors = self.interactive_node_survivors();
        self.selected_nodes.iter().any(|index| {
            survivors.contains(index) && *index > 0 && *index + 1 < self.curve.nodes.len()
        })
    }

    fn display_curve_point(&self, bounds: Rect, node: CurveNode) -> Point {
        Self::curve_point(bounds, self.display_node(node))
    }

    fn sample_display_curve(&self, phase: f32) -> f32 {
        sample_editable_curve(&self.curve, phase - self.phase_offset)
    }

    fn offset_pointer_x(bounds: Rect, position: Point) -> f32 {
        let curve_bounds = Self::curve_bounds(bounds);
        let width = (curve_bounds.width().max(1.0) - 1.0).max(1.0);
        (position.x - curve_bounds.min.x) / width
    }

    fn hit_node(&self, bounds: Rect, position: Point) -> Option<usize> {
        self.hit_node_within(bounds, position, CURVE_NODE_HIT_RADIUS)
    }

    fn hit_node_within(&self, bounds: Rect, position: Point, radius: f32) -> Option<usize> {
        if !Self::curve_bounds(bounds).contains(position) {
            return None;
        }
        let radius_squared = radius.max(0.0) * radius.max(0.0);
        let survivors = self.interactive_node_survivors();
        self.curve
            .nodes
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, node)| {
                if !survivors.contains(&index) {
                    return None;
                }
                let center = self.display_curve_point(bounds, node);
                let dx = center.x - position.x;
                let dy = center.y - position.y;
                let distance_squared = dx * dx + dy * dy;
                (distance_squared <= radius_squared).then_some((index, distance_squared))
            })
            .min_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
    }

    fn insert_node_at(&self, bounds: Rect, position: Point) -> Option<CurveNode> {
        if !Self::curve_bounds(bounds).contains(position)
            || self.curve.nodes.len() < 2
            || self.curve.nodes.len() >= MAX_EDITABLE_NODES
        {
            return None;
        }

        Some(self.raw_node_from_display_point(bounds, position))
    }

    fn hover_at(&self, bounds: Rect, position: Point) -> CurveHoverState {
        let curve_bounds = Self::curve_bounds(bounds);
        let node = if curve_bounds.contains(position) {
            self.hit_node(bounds, position)
        } else {
            None
        };
        let segment = if node.is_none() && curve_bounds.contains(position) {
            self.hit_segment(bounds, position, CURVE_SEGMENT_HOVER_RADIUS)
        } else {
            None
        };
        let preview_node =
            if (self.option_hover_held || self.command_hover_held) && segment.is_some() {
                None
            } else {
                self.preview_node_at(bounds, position, segment)
            };
        CurveHoverState {
            node,
            preview_node,
            segment,
        }
    }

    fn preview_node_at(
        &self,
        bounds: Rect,
        position: Point,
        segment: Option<usize>,
    ) -> Option<CurveNode> {
        if !Self::curve_bounds(bounds).contains(position)
            || self.curve.nodes.len() < 2
            || self.curve.nodes.len() >= MAX_EDITABLE_NODES
            || self
                .hit_node_within(bounds, position, CURVE_NODE_INSERT_GUARD_RADIUS)
                .is_some()
            || segment.is_none()
        {
            return None;
        }

        let x = self.raw_node_from_display_point(bounds, position).x;
        if x <= CURVE_NODE_MIN_SPACING_X || x >= 1.0 - CURVE_NODE_MIN_SPACING_X {
            return None;
        }
        Some(CurveNode {
            x,
            y: sample_editable_curve(&self.curve, x).clamp(0.0, 1.0),
        })
    }

    fn hit_segment(&self, bounds: Rect, position: Point, radius: f32) -> Option<usize> {
        let raw = self.raw_node_from_display_point(bounds, position);
        let curve_point = Self::curve_point(
            bounds,
            CurveNode {
                x: Self::display_phase(raw.x, self.phase_offset),
                y: sample_editable_curve(&self.curve, raw.x),
            },
        );
        let distance_squared =
            (curve_point.x - position.x).powi(2) + (curve_point.y - position.y).powi(2);
        if distance_squared > radius.max(0.0).powi(2) {
            return None;
        }
        self.curve
            .nodes
            .windows(2)
            .position(|nodes| raw.x >= nodes[0].x && raw.x <= nodes[1].x)
    }

    fn push_grid(&self, primitives: &mut Vec<PaintPrimitive>, bounds: Rect, theme: &ThemeTokens) {
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.common.id,
            rect: bounds,
            color: theme.bg_secondary,
        }));
        let curve_bounds = Self::curve_bounds(bounds);
        let grid = super::curve_beat_grid(self.sync_division, curve_bounds.width());
        for (positions, color) in [
            (grid.minor.as_slice(), theme.grid_soft),
            (grid.major.as_slice(), theme.grid_strong),
        ] {
            for position in positions {
                let position = crate::dsp::swing_warp_phase(*position, self.swing);
                let x = curve_bounds.min.x + (curve_bounds.width().max(1.0) - 1.0) * position;
                primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                    widget_id: self.common.id,
                    points: Arc::from([
                        Point::new(x, curve_bounds.min.y),
                        Point::new(x, curve_bounds.max.y),
                    ]),
                    color,
                    width: 1.0,
                }));
            }
        }
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: Arc::from([
                Point::new(curve_bounds.min.x, curve_bounds.min.y),
                Point::new(curve_bounds.min.x, curve_bounds.max.y),
            ]),
            color: theme.border_emphasis,
            width: 1.0,
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.common.id,
            rect: bounds,
            color: theme.border_emphasis,
            width: 1.0,
        }));
    }

    fn push_gain_references(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        theme: &ThemeTokens,
    ) {
        let curve_bounds = Self::curve_bounds(bounds);
        let gutter_bounds = Self::reference_gutter_bounds(bounds);
        let label_width = (gutter_bounds.width() - 6.8).max(1.0);
        let label_height = CURVE_REFERENCE_LABEL_HEIGHT.min(bounds.height().max(1.0));
        let label_left = gutter_bounds.min.x + 3.4;
        let min_top = bounds.min.y;
        let max_top = (bounds.max.y - label_height).max(min_top);

        for reference in super::curve_gain_references_for_mapping(self.depth_db, self.floor_db) {
            let y = Self::curve_point(
                bounds,
                CurveNode {
                    x: 0.0,
                    y: reference.gain,
                },
            )
            .y;
            let label_top = (y - label_height * 0.5).clamp(min_top, max_top);
            let label_rect = Rect::from_xy_size(label_left, label_top, label_width, label_height);
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.common.id,
                points: Arc::from([
                    Point::new(curve_bounds.min.x, y),
                    Point::new(curve_bounds.max.x, y),
                ]),
                color: theme.text_muted.with_alpha(72),
                width: 1.0,
            }));
            primitives.push(PaintPrimitive::Text(PaintTextRun {
                widget_id: self.common.id,
                text: PaintText::from(super::curve_gain_reference_text(reference, false)),
                rect: label_rect,
                font_size: CURVE_REFERENCE_FONT_SIZE,
                baseline: None,
                color: theme.text_muted,
                align: PaintTextAlign::Right,
                wrap: TextWrap::None,
            }));
        }
    }

    fn push_curve(&self, primitives: &mut Vec<PaintPrimitive>, bounds: Rect, theme: &ThemeTokens) {
        let curve_bounds = Self::curve_bounds(bounds);
        let points = self.sample_curve_points(bounds);
        let fill_points = points.clone();
        let fill_steps = fill_points.len().saturating_sub(1).max(1);
        let gradient = PaintLinearGradient::vertical(
            curve_bounds,
            theme.accent_mint.with_alpha(CURVE_FILL_TOP_ALPHA),
            theme.accent_mint.with_alpha(CURVE_FILL_BOTTOM_ALPHA),
        );
        push_sampled_curve_area_fill(
            primitives,
            SampledCurveAreaFillParts::new(
                self.common.id,
                curve_bounds,
                fill_steps,
                SampledCurveAreaBaseline::Bottom,
                PaintBrush::linear_gradient(gradient),
            ),
            move |phase| {
                let index = (phase * fill_steps as f32).round() as usize;
                fill_points.get(index).copied()
            },
        );
        if self.smooth > f32::EPSILON {
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.common.id,
                points: Arc::from(self.sample_smoothed_curve_points(bounds)),
                color: theme.text_primary.with_alpha(188),
                width: 1.275,
            }));
        }
        let active_offset = self.active_curve_offset_start_x.is_some();
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: Arc::from(points.clone()),
            color: if active_offset {
                CURVE_OFFSET_MOVE_COLOR
            } else {
                theme.accent_mint
            },
            width: if active_offset { 2.55 } else { 1.7 },
        }));
        if self.command_hover_held && self.shift_hover_held && !active_offset {
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.common.id,
                points: Arc::from(points.clone()),
                color: CURVE_OFFSET_HOVER_COLOR,
                width: 2.975,
            }));
        }

        let move_segment = (self.active_segment_move && self.active_segment.is_some())
            .then_some(self.active_segment)
            .flatten()
            .or_else(|| {
                self.command_hover_held
                    .then_some(self.hover_segment)
                    .flatten()
            });
        let tension_segment = (!self.active_segment_move)
            .then_some(self.active_segment)
            .flatten()
            .or_else(|| {
                (!self.command_hover_held && self.option_hover_held)
                    .then_some(self.hover_segment)
                    .flatten()
            });
        if let Some((segment, color)) = move_segment
            .map(|segment| (segment, CURVE_SEGMENT_MOVE_COLOR))
            .or_else(|| tension_segment.map(|segment| (segment, theme.accent_warning)))
        {
            for points in self.sample_segment_polylines(bounds, segment) {
                if points.len() <= 1 {
                    continue;
                }
                primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                    widget_id: self.common.id,
                    points: Arc::from(points),
                    color,
                    width: 2.975,
                }));
            }
        }
        self.push_curve_paint_preview(primitives, bounds, theme);
    }

    fn push_curve_paint_preview(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        theme: &ThemeTokens,
    ) {
        if !self.active_curve_paint {
            return;
        }
        for run in &self.curve_paint_runs {
            if run.points().len() < 2 {
                continue;
            }
            let points: Vec<Point> = run
                .points()
                .iter()
                .map(|point| {
                    Self::curve_point(
                        bounds,
                        CurveNode {
                            x: point.position.x,
                            y: point.position.y,
                        },
                    )
                })
                .collect();
            primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                widget_id: self.common.id,
                points: Arc::from(points),
                color: theme.accent_copper,
                width: CURVE_PAINT_PREVIEW_WIDTH,
            }));
        }
    }

    fn push_offset_bar(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        theme: &ThemeTokens,
    ) {
        let bar = Self::offset_bar_bounds(bounds);
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.common.id,
            rect: bar,
            color: theme.surface_raised,
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.common.id,
            rect: bar,
            color: theme.border,
            width: 1.0,
        }));
        primitives.push(PaintPrimitive::Text(PaintTextRun {
            widget_id: self.common.id,
            text: PaintText::from_static("OFFSET"),
            rect: Rect::from_xy_size(
                bounds.min.x,
                bar.min.y,
                (bar.min.x - bounds.min.x - CURVE_OFFSET_BAR_INSET).max(1.0),
                bar.height(),
            ),
            font_size: PUMP_TYPOGRAPHY.meta.0,
            baseline: None,
            color: theme.text_muted,
            align: PaintTextAlign::Right,
            wrap: TextWrap::None,
        }));
        let handle_x =
            bar.min.x + self.phase_offset * (bar.width() - CURVE_OFFSET_HANDLE_WIDTH).max(0.0);
        let handle = Rect::from_xy_size(
            handle_x,
            bar.min.y,
            CURVE_OFFSET_HANDLE_WIDTH.min(bar.width()),
            bar.height(),
        );
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.common.id,
            rect: handle,
            color: theme.accent_mint,
        }));
    }

    fn push_incoming_waveform(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        theme: &ThemeTokens,
    ) {
        let Some(waveform) = self.incoming_waveform.as_ref() else {
            return;
        };
        self.push_waveform_layer(
            primitives,
            bounds,
            waveform,
            theme.text_muted.with_alpha(88),
            1.0,
        );
    }

    fn processed_waveform(&self) -> Option<IncomingWaveformSnapshot> {
        self.incoming_waveform.as_ref().map(|waveform| {
            std::array::from_fn(|index| {
                let viewport_phase = index as f32 / (waveform.len() - 1) as f32;
                let gain = crate::dsp::curve_value_to_gain(
                    self.sample_display_curve(viewport_phase),
                    self.depth_db,
                    self.floor_db,
                );
                let input = if waveform[index].is_finite() {
                    waveform[index]
                } else {
                    0.0
                };
                (input * gain).clamp(0.0, 1.0)
            })
        })
    }

    fn push_processed_waveform(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        theme: &ThemeTokens,
    ) {
        let Some(waveform) = self.processed_waveform() else {
            return;
        };
        self.push_waveform_layer(
            primitives,
            bounds,
            &waveform,
            theme.accent_copper.with_alpha(96),
            2.0,
        );
    }

    fn push_waveform_layer(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        waveform: &[f32],
        color: Rgba8,
        width: f32,
    ) {
        let curve_bounds = Self::curve_bounds(bounds);
        let center_y = curve_bounds.min.y + curve_bounds.height() * 0.5;
        let amplitude_scale = curve_bounds.height() * 0.43;
        let points_for = |upper: bool| -> Arc<[Point]> {
            Arc::from(
                waveform
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(index, amplitude)| {
                        let phase = index as f32 / (waveform.len() - 1) as f32;
                        let x = curve_bounds.min.x + phase * (curve_bounds.width().max(1.0) - 1.0);
                        let offset = amplitude.clamp(0.0, 1.0) * amplitude_scale;
                        Point::new(x, center_y + if upper { -offset } else { offset })
                    })
                    .collect::<Vec<_>>(),
            )
        };
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: points_for(true),
            color,
            width,
        }));
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: points_for(false),
            color,
            width,
        }));
    }

    fn sample_curve_points(&self, bounds: Rect) -> Vec<Point> {
        let curve_bounds = Self::curve_bounds(bounds);
        let width = (curve_bounds.width().max(1.0) - 1.0).max(1.0);
        let mut samples = Vec::new();

        // Keep authored nodes in the rendered polyline. A uniform phase grid
        // can otherwise land on neither side of a narrow or steep segment.
        for node in self.curve.nodes.iter().copied() {
            let display = self.display_node(node);
            samples.push((display.x, Self::curve_point(bounds, display)));
        }

        // Sample each authored segment in proportion to its displayed width.
        // Interior samples avoid duplicating the explicit node points above.
        for nodes in self.curve.nodes.windows(2) {
            let left = nodes[0];
            let right = nodes[1];
            let span = (right.x - left.x).max(0.0);
            let steps = (span * width).ceil().clamp(2.0, CURVE_SAMPLE_COUNT as f32) as usize;
            for step in 1..steps {
                let t = step as f32 / steps as f32;
                let raw_x = left.x + (right.x - left.x) * t;
                let display = self.display_node(CurveNode {
                    x: raw_x,
                    y: sample_editable_curve(&self.curve, raw_x),
                });
                samples.push((display.x, Self::curve_point(bounds, display)));
            }
        }

        // Always cover both viewport edges. The curve is cyclic, so these are
        // equivalent when the phase offset puts the seam in the middle.
        for phase in [0.0, 1.0] {
            samples.push((
                phase,
                Self::curve_point(
                    bounds,
                    CurveNode {
                        x: phase,
                        y: self.sample_display_curve(phase),
                    },
                ),
            ));
        }

        samples.sort_by(|left, right| left.0.total_cmp(&right.0));
        let mut points = Vec::with_capacity(samples.len());
        let mut last_phase: Option<f32> = None;
        for (phase, point) in samples {
            // Preserve the distinct 0 and 1 endpoints while collapsing the
            // duplicate points introduced at segment boundaries and the seam.
            let duplicate = last_phase.is_some_and(|last| {
                (phase - last).abs() <= 1.0e-6 && phase > 1.0e-6 && phase < 1.0 - 1.0e-6
            });
            if !duplicate {
                points.push(point);
                last_phase = Some(phase);
            }
        }
        points
    }

    fn sample_smoothed_curve(&self, phase: f32) -> f32 {
        if self.smooth <= f32::EPSILON {
            return sample_editable_curve(&self.curve, phase);
        }
        let radius = Self::smooth_preview_radius(self.smooth);
        let sample_step = 1.0 / CURVE_SAMPLE_COUNT as f32;
        let mut total = 0.0;
        let mut count = 0.0;
        for offset in -radius..=radius {
            total += sample_editable_curve(&self.curve, phase + offset as f32 * sample_step);
            count += 1.0;
        }
        (total / count).clamp(0.0, 1.0)
    }

    fn smooth_preview_radius(amount: f32) -> i32 {
        let amount = if amount.is_finite() {
            amount.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if amount <= crate::dsp::SMOOTH_COMPATIBILITY_KNEE {
            return (amount * 8.0).round() as i32;
        }

        let t = (amount - crate::dsp::SMOOTH_COMPATIBILITY_KNEE)
            / (1.0 - crate::dsp::SMOOTH_COMPATIBILITY_KNEE);
        let smoothstep = t * t * (3.0 - 2.0 * t);
        (amount * 8.0 + (20.0 - 8.0) * smoothstep).round() as i32
    }

    fn sample_smoothed_curve_points(&self, bounds: Rect) -> Vec<Point> {
        (0..=CURVE_SAMPLE_COUNT)
            .map(|step| {
                let phase = step as f32 / CURVE_SAMPLE_COUNT as f32;
                Self::curve_point(
                    bounds,
                    CurveNode {
                        x: phase,
                        y: self.sample_smoothed_curve(phase - self.phase_offset),
                    },
                )
            })
            .collect()
    }

    fn sample_segment_points(&self, bounds: Rect, index: usize) -> Vec<Point> {
        let Some(left) = self.curve.nodes.get(index).copied() else {
            return Vec::new();
        };
        let Some(right) = self.curve.nodes.get(index + 1).copied() else {
            return Vec::new();
        };
        let curve_bounds = Self::curve_bounds(bounds);
        let width = (curve_bounds.width().max(1.0) - 1.0).max(1.0);
        let steps = ((right.x - left.x).max(0.0) * width)
            .ceil()
            .clamp(2.0, CURVE_SAMPLE_COUNT as f32) as usize;
        let mut points = Vec::with_capacity(steps + 1);
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let x = left.x + (right.x - left.x) * t;
            points.push(self.display_curve_point(
                bounds,
                CurveNode {
                    x,
                    y: sample_editable_curve(&self.curve, x),
                },
            ));
        }
        points
    }

    fn sample_segment_polylines(&self, bounds: Rect, index: usize) -> Vec<Vec<Point>> {
        let points = self.sample_segment_points(bounds, index);
        let Some(first) = points.first().copied() else {
            return Vec::new();
        };

        let mut polylines = vec![vec![first]];
        for point in points.into_iter().skip(1) {
            let current = polylines
                .last_mut()
                .expect("segment polyline list always has a current line");
            let previous = current
                .last()
                .copied()
                .expect("segment polyline always has a previous point");
            if point.x + 1.0e-6 < previous.x {
                polylines.push(vec![point]);
            } else {
                current.push(point);
            }
        }
        polylines
    }

    fn push_nodes(&self, primitives: &mut Vec<PaintPrimitive>, bounds: Rect, theme: &ThemeTokens) {
        if let Some(preview) = self.preview_node {
            let center = self.display_curve_point(bounds, preview);
            let radius = CURVE_PREVIEW_NODE_SIZE * 0.5;
            let rect = Rect::from_xy_size(
                center.x - radius,
                center.y - radius,
                CURVE_PREVIEW_NODE_SIZE,
                CURVE_PREVIEW_NODE_SIZE,
            );
            primitives.push(PaintPrimitive::FillRect(PaintFillRect {
                widget_id: self.common.id,
                rect,
                color: theme.accent_warning,
            }));
            primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
                widget_id: self.common.id,
                rect,
                color: theme.accent_mint,
                width: 1.0,
            }));
        }

        let survivors = self.interactive_node_survivors();
        for (index, node) in self.curve.nodes.iter().copied().enumerate() {
            if !survivors.contains(&index) {
                continue;
            }
            let center = self.display_curve_point(bounds, node);
            let active = self.active_node == Some(index);
            let selected = self.selected_nodes.contains(&index);
            let hovered = self.hover_node == Some(index);
            let size = if active || selected {
                CURVE_NODE_SIZE + 1.7
            } else if hovered {
                CURVE_NODE_SIZE + 1.275
            } else {
                CURVE_NODE_SIZE
            };
            let radius = size * 0.5;
            let rect = Rect::from_xy_size(center.x - radius, center.y - radius, size, size);
            primitives.push(PaintPrimitive::FillRect(PaintFillRect {
                widget_id: self.common.id,
                rect,
                color: if active || selected {
                    theme.accent_warning
                } else if hovered {
                    theme.accent_mint
                } else {
                    theme.surface_raised
                },
            }));
            primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
                widget_id: self.common.id,
                rect,
                color: if selected || (active && hovered) {
                    theme.accent_mint
                } else if hovered {
                    theme.accent_warning
                } else {
                    theme.accent_copper
                },
                width: if hovered { 1.275 } else { 1.0 },
            }));
        }
    }

    fn push_marquee(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        theme: &ThemeTokens,
    ) {
        let Some(marquee) = self.active_marquee else {
            return;
        };
        let start = self.display_curve_point(bounds, marquee.start);
        let current = self.display_curve_point(bounds, marquee.current);
        let rect = Rect::from_xy_size(
            start.x.min(current.x),
            start.y.min(current.y),
            (start.x - current.x).abs(),
            (start.y - current.y).abs(),
        );
        primitives.push(PaintPrimitive::FillRect(PaintFillRect {
            widget_id: self.common.id,
            rect,
            color: theme.accent_mint.with_alpha(32),
        }));
        primitives.push(PaintPrimitive::StrokeRect(PaintStrokeRect {
            widget_id: self.common.id,
            rect,
            color: theme.accent_mint,
            width: 1.0,
        }));
    }

    fn push_playhead(&self, primitives: &mut Vec<PaintPrimitive>, bounds: Rect) {
        let Some(phase) = self.playhead_phase else {
            return;
        };
        let sample = self.sample_display_curve(phase).clamp(0.0, 1.0);
        let center = Self::curve_point(
            bounds,
            CurveNode {
                x: phase,
                y: sample,
            },
        );
        primitives.push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
            widget_id: self.common.id,
            points: [
                Point::new(center.x, bounds.min.y),
                Point::new(center.x, bounds.max.y),
            ]
            .into(),
            color: CURVE_PLAYHEAD_CORE_COLOR,
            width: 1.275,
        }));
        primitives.push(PaintPrimitive::FillPolygon(PaintFillPolygon {
            widget_id: self.common.id,
            points: [
                Point::new(center.x - CURVE_PLAYHEAD_MARKER_WIDTH * 0.5, bounds.min.y),
                Point::new(center.x + CURVE_PLAYHEAD_MARKER_WIDTH * 0.5, bounds.min.y),
                Point::new(center.x, bounds.min.y + CURVE_PLAYHEAD_MARKER_HEIGHT),
            ]
            .into(),
            color: CURVE_PLAYHEAD_CORE_COLOR,
        }));
    }
}

impl Widget for CurvePreviewWidget {
    fn common(&self) -> &WidgetCommon {
        &self.common
    }

    fn common_mut(&mut self) -> &mut WidgetCommon {
        &mut self.common
    }

    fn handle_input(&mut self, bounds: Rect, input: WidgetInput) -> Option<WidgetOutput> {
        let message = match input {
            // Radiant projects both right-click and macOS Control-click into
            // the secondary gesture, so both paths share the paint behavior.
            WidgetInput::PointerPress {
                position,
                button: PointerButton::Secondary,
                ..
            } if Self::curve_bounds(bounds).contains(position) => {
                Some(CurvePreviewMessage::PressPaint {
                    sample: self.paint_sample_from_display_point(bounds, position),
                })
            }
            WidgetInput::PointerPress {
                position,
                button: PointerButton::Primary,
                modifiers,
            } => {
                let command_held = self.command_hover_held || modifiers.command;
                let option_held = self.option_hover_held || modifiers.alt;
                let shift_held = modifiers.shift;
                let offset_gesture = modifiers.command
                    && modifiers.shift
                    && Self::curve_bounds(bounds).contains(position);
                let hit_node = self.hit_node(bounds, position);
                if Self::offset_bar_bounds(bounds).contains(position) || offset_gesture {
                    Some(CurvePreviewMessage::PressCurveOffset {
                        pointer_x: Self::offset_pointer_x(bounds, position),
                        quantized: option_held,
                    })
                } else if let Some(index) = hit_node {
                    Some(CurvePreviewMessage::PressNode {
                        index,
                        pointer: self.raw_node_from_display_point(bounds, position),
                        shift_held,
                        option_held,
                        command_held,
                    })
                } else if shift_held
                    && !option_held
                    && Self::curve_bounds(bounds).contains(position)
                {
                    Some(CurvePreviewMessage::PressMarquee {
                        start: self.raw_node_from_display_point(bounds, position),
                    })
                } else {
                    let hover = self.hover_at(bounds, position);
                    match (command_held, option_held, hover.segment) {
                        (true, _, Some(index)) => {
                            Some(CurvePreviewMessage::PressSegmentMove { index, position })
                        }
                        (false, true, Some(index)) => {
                            Some(CurvePreviewMessage::PressSegment { index, position })
                        }
                        (false, true, None) => None,
                        _ => hover
                            .preview_node
                            .or_else(|| self.insert_node_at(bounds, position))
                            .map(|node| CurvePreviewMessage::InsertNode {
                                node: if command_held {
                                    CurveNode {
                                        x: snap_curve_time_to_beat_grid_with_swing(
                                            self.sync_division,
                                            Self::curve_bounds(bounds).width(),
                                            node.x,
                                            self.swing,
                                        ),
                                        ..node
                                    }
                                } else {
                                    node
                                },
                                command_held,
                            }),
                    }
                }
            }
            WidgetInput::PointerDoubleClick {
                position,
                button: PointerButton::Primary,
                ..
            } => {
                if Self::offset_bar_bounds(bounds).contains(position) {
                    Some(CurvePreviewMessage::ResetCurveOffset)
                } else {
                    self.hit_node(bounds, position)
                        .filter(|index| *index > 0 && *index + 1 < self.curve.nodes.len())
                        .map(|index| CurvePreviewMessage::DeleteNode { index })
                }
            }
            WidgetInput::KeyPress(WidgetKey::Delete | WidgetKey::Backspace)
                if self.common.state.focused && self.has_interactive_deletable_selection() =>
            {
                Some(CurvePreviewMessage::DeleteSelectedNodes)
            }
            WidgetInput::PointerMove { position } => {
                if self.active_marquee.is_some() {
                    Some(CurvePreviewMessage::DragMarquee {
                        current: self.raw_node_from_display_point(bounds, position),
                    })
                } else if self.active_curve_paint {
                    if Self::curve_bounds(bounds).contains(position) {
                        Some(CurvePreviewMessage::DragPaint {
                            sample: self.paint_sample_from_display_point(bounds, position),
                        })
                    } else {
                        Some(CurvePreviewMessage::DragPaintOutside {
                            sample: self.paint_sample_from_boundary(bounds, position),
                        })
                    }
                } else if let Some(index) = self.active_node {
                    Some(CurvePreviewMessage::DragNode {
                        index,
                        node: self.raw_node_from_display_point(bounds, position),
                        push_through_threshold_x: curve_node_push_through_threshold_x(
                            bounds.width(),
                        ),
                    })
                } else if let Some(start_x) = self.active_curve_offset_start_x {
                    Some(CurvePreviewMessage::DragCurveOffset {
                        delta: Self::offset_pointer_x(bounds, position) - start_x,
                    })
                } else if let Some(index) = self.active_segment {
                    Some(CurvePreviewMessage::DragSegment {
                        index,
                        position,
                        curve_size: Vector2::new(
                            Self::curve_bounds(bounds).width(),
                            Self::curve_bounds(bounds).height(),
                        ),
                    })
                } else {
                    let hover = self.hover_at(bounds, position);
                    (hover.node != self.hover_node
                        || hover.preview_node != self.preview_node
                        || hover.segment != self.hover_segment)
                        .then_some(CurvePreviewMessage::Hover {
                            node: hover.node,
                            preview_node: hover.preview_node,
                            segment: hover.segment,
                        })
                }
            }
            WidgetInput::PointerModifiersChanged { modifiers } => (modifiers.alt
                != self.option_hover_held
                || modifiers.command != self.command_hover_held
                || modifiers.shift != self.shift_hover_held)
                .then_some(CurvePreviewMessage::ModifiersChanged {
                    option_held: modifiers.alt,
                    command_held: modifiers.command,
                    shift_held: modifiers.shift,
                }),
            WidgetInput::PointerRelease {
                position,
                button: PointerButton::Secondary,
                ..
            }
            | WidgetInput::PointerDrop {
                position,
                button: PointerButton::Secondary,
                ..
            } if self.active_curve_paint => {
                if Self::curve_bounds(bounds).contains(position) {
                    Some(CurvePreviewMessage::ReleasePaint {
                        sample: Some(self.paint_sample_from_display_point(bounds, position)),
                    })
                } else {
                    Some(CurvePreviewMessage::ReleasePaintOutside {
                        sample: self.paint_sample_from_boundary(bounds, position),
                    })
                }
            }
            WidgetInput::PointerRelease {
                position,
                button: PointerButton::Primary,
                modifiers,
            }
            | WidgetInput::PointerDrop {
                position,
                button: PointerButton::Primary,
                modifiers,
            } => {
                if self.active_marquee.is_some() {
                    Some(CurvePreviewMessage::ReleaseMarquee {
                        current: self.raw_node_from_display_point(bounds, position),
                    })
                } else if let Some(index) = self.active_node {
                    Some(CurvePreviewMessage::ReleaseNode {
                        index,
                        node: self.raw_node_from_display_point(bounds, position),
                        push_through_threshold_x: curve_node_push_through_threshold_x(
                            bounds.width(),
                        ),
                        shift_held: modifiers.shift,
                        option_held: modifiers.alt,
                        command_held: modifiers.command,
                    })
                } else if let Some(start_x) = self.active_curve_offset_start_x {
                    Some(CurvePreviewMessage::ReleaseCurveOffset {
                        delta: Self::offset_pointer_x(bounds, position) - start_x,
                        option_held: modifiers.alt,
                    })
                } else {
                    self.active_segment
                        .map(|index| CurvePreviewMessage::ReleaseSegment {
                            index,
                            position,
                            curve_size: Vector2::new(
                                Self::curve_bounds(bounds).width(),
                                Self::curve_bounds(bounds).height(),
                            ),
                        })
                }
            }
            WidgetInput::FocusChanged(focused) => {
                self.common.state.focused = focused;
                (!focused
                    && (self.active_node.is_some()
                        || self.active_segment.is_some()
                        || self.active_curve_paint
                        || self.active_curve_offset_start_x.is_some()
                        || self.active_marquee.is_some()
                        || self.hover_node.is_some()
                        || self.preview_node.is_some()
                        || self.hover_segment.is_some()
                        || self.option_hover_held
                        || self.command_hover_held
                        || self.shift_hover_held))
                    .then_some(CurvePreviewMessage::Cancel)
            }
            _ => None,
        }?;
        Some(WidgetOutput::typed(message))
    }

    fn append_paint(
        &self,
        primitives: &mut Vec<PaintPrimitive>,
        bounds: Rect,
        _layout: &LayoutOutput,
        theme: &ThemeTokens,
    ) {
        self.push_grid(primitives, bounds, theme);
        self.push_processed_waveform(primitives, bounds, theme);
        self.push_incoming_waveform(primitives, bounds, theme);
        self.push_gain_references(primitives, bounds, theme);
        self.push_curve(primitives, bounds, theme);
        self.push_offset_bar(primitives, bounds, theme);
        self.push_marquee(primitives, bounds, theme);
        self.push_nodes(primitives, bounds, theme);
        self.push_playhead(primitives, bounds);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CurvePreviewMessage {
    Hover {
        node: Option<usize>,
        preview_node: Option<CurveNode>,
        segment: Option<usize>,
    },
    ModifiersChanged {
        option_held: bool,
        command_held: bool,
        shift_held: bool,
    },
    PressNode {
        index: usize,
        pointer: CurveNode,
        shift_held: bool,
        option_held: bool,
        command_held: bool,
    },
    PressPaint {
        sample: CurvePaintSample,
    },
    DragPaint {
        sample: CurvePaintSample,
    },
    DragPaintOutside {
        sample: CurvePaintSample,
    },
    ReleasePaint {
        sample: Option<CurvePaintSample>,
    },
    ReleasePaintOutside {
        sample: CurvePaintSample,
    },
    PressMarquee {
        start: CurveNode,
    },
    DragMarquee {
        current: CurveNode,
    },
    ReleaseMarquee {
        current: CurveNode,
    },
    PressCurveOffset {
        pointer_x: f32,
        quantized: bool,
    },
    ResetCurveOffset,
    InsertNode {
        node: CurveNode,
        command_held: bool,
    },
    DeleteNode {
        index: usize,
    },
    DeleteSelectedNodes,
    DragNode {
        index: usize,
        node: CurveNode,
        push_through_threshold_x: f32,
    },
    DragCurveOffset {
        delta: f32,
    },
    ReleaseNode {
        index: usize,
        node: CurveNode,
        push_through_threshold_x: f32,
        shift_held: bool,
        option_held: bool,
        command_held: bool,
    },
    ReleaseCurveOffset {
        delta: f32,
        option_held: bool,
    },
    PressSegment {
        index: usize,
        position: Point,
    },
    PressSegmentMove {
        index: usize,
        position: Point,
    },
    DragSegment {
        index: usize,
        position: Point,
        curve_size: Vector2,
    },
    ReleaseSegment {
        index: usize,
        position: Point,
        curve_size: Vector2,
    },
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CurveHoverState {
    node: Option<usize>,
    preview_node: Option<CurveNode>,
    segment: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::super::curve_paint::{BoundaryContact, BoundaryEdge, EdgeParameter};
    use super::*;
    use crate::curve::sample_curve_segment;
    #[cfg(feature = "vst3")]
    use crate::GuiTransportTelemetry;
    use radiant::runtime::PaintPrimitive;
    use radiant::widgets::PointerModifiers;
    use toybox::clack_plugin::events::event_types::{
        ParamGestureBeginEvent, ParamGestureEndEvent, ParamValueEvent,
    };
    use toybox::clack_plugin::events::io::EventBuffer;
    use toybox::clack_plugin::events::spaces::CoreEventSpace;
    use toybox::clack_plugin::events::Event;
    use toybox::clap::automation::{AutomationDropPolicy, AutomationQueueConfig};

    const CURVE_PAINT_ASSERT_EPSILON: f32 = 1.0e-4;

    fn editor_state(params: Arc<PumpParams>) -> RadiantEditorState {
        RadiantEditorState::new(
            params,
            Arc::new(GuiStatus::default()),
            Arc::new(ClapHostParamEditSink {
                queue: Arc::new(PumpAutomationQueue::default()),
                requester: None,
            }),
        )
    }

    fn paint_sample(x: f32, y: f32) -> CurvePaintSample {
        CurvePaintSample {
            node: CurveNode { x, y },
            display_position: RectPoint { x, y },
            outside: false,
        }
    }

    fn boundary_paint_sample(x: f32, y: f32) -> CurvePaintSample {
        CurvePaintSample {
            node: CurveNode { x, y },
            display_position: RectPoint { x, y },
            outside: true,
        }
    }

    fn recorded_run(points: impl IntoIterator<Item = RectPoint>) -> PaintRun {
        let mut recorder = StrokeRecorder::new(RectBounds {
            min: RectPoint { x: 0.0, y: 0.0 },
            max: RectPoint { x: 1.0, y: 1.0 },
        });
        for point in points {
            recorder.observe(point);
        }
        assert_eq!(recorder.runs().len(), 1);
        recorder
            .runs()
            .first()
            .cloned()
            .expect("recorded points should produce one paint run")
    }

    fn sampled_segment_run(
        left: CurveNode,
        right: CurveNode,
        tension: f32,
        steps: usize,
    ) -> PaintRun {
        recorded_run((0..=steps).map(|step| {
            let fraction = step as f32 / steps as f32;
            let x = left.x + (right.x - left.x) * fraction;
            RectPoint {
                x,
                y: sample_curve_segment(left, right, tension, x),
            }
        }))
    }

    fn assert_curve_paint_topology_is_bounded(curve: &EditableCurve) {
        assert!(curve.nodes.len() <= MAX_EDITABLE_NODES);
        assert_eq!(curve.segments.len(), curve.nodes.len().saturating_sub(1));
        assert_eq!(curve.nodes.first().map(|node| node.x), Some(0.0));
        assert_eq!(curve.nodes.last().map(|node| node.x), Some(1.0));
        assert!(curve.nodes.iter().all(|node| {
            node.x.is_finite()
                && node.y.is_finite()
                && (0.0..=1.0).contains(&node.x)
                && (0.0..=1.0).contains(&node.y)
        }));
        assert!(curve
            .nodes
            .windows(2)
            .all(|pair| pair[1].x - pair[0].x >= CURVE_PAINT_ASSERT_EPSILON));
    }

    fn test_curve_push_through_threshold_x() -> f32 {
        curve_node_push_through_threshold_x(300.0)
    }

    fn unconstrained_press(index: usize) -> CurvePreviewMessage {
        CurvePreviewMessage::PressNode {
            index,
            pointer: CurveNode { x: 0.0, y: 0.0 },
            shift_held: false,
            option_held: false,
            command_held: false,
        }
    }

    fn legacy_edge_curve() -> EditableCurve {
        EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.75 },
                CurveNode { x: 0.0001, y: 0.2 },
                CurveNode { x: 0.5, y: 0.45 },
                CurveNode { x: 0.9999, y: 0.15 },
                CurveNode { x: 1.0, y: 0.75 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],
            ..EditableCurve::default()
        }
        .normalized()
    }

    fn painted_node_centers(widget: &CurvePreviewWidget, bounds: Rect) -> Vec<Point> {
        let mut primitives = Vec::new();
        widget.append_paint(
            &mut primitives,
            bounds,
            &LayoutOutput::default(),
            &ThemeTokens::default(),
        );
        primitives
            .iter()
            .filter_map(|primitive| match primitive {
                PaintPrimitive::FillRect(fill)
                    if fill.rect.width() > CURVE_NODE_SIZE - 1.0e-5
                        && fill.rect.width() < CURVE_NODE_SIZE + 1.7 + 1.0e-3
                        && (fill.rect.width() - fill.rect.height()).abs() < 1.0e-3 =>
                {
                    Some(fill.rect.center())
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn legacy_edge_nodes_have_one_painted_hit_and_marquee_survivor_per_side() {
        let curve = legacy_edge_curve();
        assert_eq!(curve.nodes.len(), 5);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let survivors = interactive_curve_node_survivors(&curve, 0.0, None);
        assert_eq!(survivors, vec![0, 2, 4]);

        let painted = painted_node_centers(&widget, bounds);
        assert_eq!(painted.len(), survivors.len());
        for index in survivors.iter().copied() {
            let center = widget.display_curve_point(bounds, curve.nodes[index]);
            assert!(painted.iter().any(|painted| {
                (painted.x - center.x).abs() < 1.0e-5 && (painted.y - center.y).abs() < 1.0e-5
            }));
            assert_eq!(widget.hit_node(bounds, center), Some(index));
        }
        for index in [1, 3] {
            let center = widget.display_curve_point(bounds, curve.nodes[index]);
            assert!(!painted.iter().any(|painted| {
                (painted.x - center.x).abs() < 1.0e-5 && (painted.y - center.y).abs() < 1.0e-5
            }));
            assert_eq!(widget.hit_node(bounds, center), None);
        }

        let params = Arc::new(PumpParams::new());
        params.set_editable_curve(&curve);
        let before = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressMarquee {
                start: CurveNode { x: 0.0, y: 1.0 },
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ReleaseMarquee {
                current: CurveNode { x: 1.0, y: 0.0 },
            },
        );
        assert_eq!(state.selected_curve_nodes, survivors);
        assert_eq!(params.editable_curve_snapshot(), before);
    }

    #[test]
    fn legacy_edge_active_node_wins_deterministically_for_paint_and_hit() {
        let curve = legacy_edge_curve();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);

        let left_active =
            CurvePreviewWidget::new(curve.clone(), Some(1), None, None, None, None, false);
        assert_eq!(
            interactive_curve_node_survivors(&curve, 0.0, Some(1)),
            vec![1, 2, 4]
        );
        let left_center = left_active.display_curve_point(bounds, curve.nodes[1]);
        assert_eq!(left_active.hit_node(bounds, left_center), Some(1));
        assert!(painted_node_centers(&left_active, bounds)
            .iter()
            .any(|center| *center == left_center));

        let right_active =
            CurvePreviewWidget::new(curve.clone(), Some(3), None, None, None, None, false);
        assert_eq!(
            interactive_curve_node_survivors(&curve, 0.0, Some(3)),
            vec![0, 2, 3]
        );
        let right_center = right_active.display_curve_point(bounds, curve.nodes[3]);
        assert_eq!(right_active.hit_node(bounds, right_center), Some(3));
        let right_painted = painted_node_centers(&right_active, bounds);
        assert!(right_painted.iter().any(|center| {
            (center.x - right_center.x).abs() < 1.0e-5 && (center.y - right_center.y).abs() < 1.0e-5
        }));
    }

    #[test]
    fn legacy_edge_hidden_selection_is_not_deleted() {
        let params = Arc::new(PumpParams::new());
        let curve = legacy_edge_curve();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));
        state.selected_curve_nodes = (0..curve.nodes.len()).collect();

        reduce_curve_message(&mut state, CurvePreviewMessage::DeleteSelectedNodes);

        let remaining = params.editable_curve_snapshot();
        assert_eq!(remaining.nodes.len(), 4);
        assert!(remaining
            .nodes
            .iter()
            .any(|node| (node.x - 0.0001).abs() < 1.0e-6));
        assert!(remaining
            .nodes
            .iter()
            .any(|node| (node.x - 0.9999).abs() < 1.0e-6));
        assert!(!remaining
            .nodes
            .iter()
            .any(|node| (node.x - 0.5).abs() < 1.0e-6));
    }

    #[test]
    fn legacy_edge_node_drag_preserves_origin_for_vertical_movement() {
        let params = Arc::new(PumpParams::new());
        let curve = legacy_edge_curve();
        params.set_editable_curve(&curve);
        params.set_phase_offset(0.0001);
        let origin = curve.nodes[3];
        let mut state = editor_state(Arc::clone(&params));

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 3,
                pointer: origin,
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );
        assert_eq!(state.active_curve_node, Some(3));
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 3,
                node: CurveNode {
                    x: origin.x,
                    y: 0.35,
                },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), curve.nodes.len());
        assert!((dragged.nodes[3].x - origin.x).abs() < 1.0e-6);
        assert!((dragged.nodes[3].y - 0.35).abs() < 1.0e-6);
    }

    #[test]
    fn legacy_edge_group_drag_keeps_zero_delta_feasible_for_vertical_movement() {
        let params = Arc::new(PumpParams::new());
        let curve = legacy_edge_curve();
        params.set_editable_curve(&curve);
        params.set_phase_offset(0.0001);
        let origin = curve.nodes[3];
        let mut state = editor_state(Arc::clone(&params));
        state.selected_curve_nodes = vec![0, 3];

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 3,
                pointer: origin,
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );
        assert_eq!(
            state
                .active_curve_node_drag
                .as_ref()
                .map(|drag| drag.selected_indices.clone()),
            Some(vec![0, 3])
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 3,
                node: CurveNode {
                    x: origin.x,
                    y: 0.35,
                },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), curve.nodes.len());
        assert!((dragged.nodes[3].x - origin.x).abs() < 1.0e-6);
        assert!((dragged.nodes[3].y - 0.35).abs() < 1.0e-6);
        assert!((dragged.nodes[0].y - 0.95).abs() < 1.0e-6);
        assert!((dragged.nodes[4].y - 0.95).abs() < 1.0e-6);
    }

    #[test]
    fn ab_copy_and_switch_are_coherent_undo_redo_actions() {
        let params = Arc::new(PumpParams::new());
        params.set_mix(0.2);
        let mut state = editor_state(Arc::clone(&params));
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::SelectSound {
                side: SoundSide::B,
                copy: true,
            },
        );
        assert!(!params.sound_sides_differ());
        reduce_editor_message(&mut state, RadiantEditorMessage::Undo);
        assert!(params.sound_sides_differ());
        reduce_editor_message(&mut state, RadiantEditorMessage::Redo);
        assert!(!params.sound_sides_differ());
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::SelectSound {
                side: SoundSide::B,
                copy: false,
            },
        );
        assert_eq!(params.active_sound(), SoundSide::B);
        reduce_editor_message(&mut state, RadiantEditorMessage::Undo);
        assert_eq!(params.active_sound(), SoundSide::A);
        reduce_editor_message(&mut state, RadiantEditorMessage::Redo);
        assert_eq!(params.active_sound(), SoundSide::B);
    }

    #[test]
    fn ab_command_copy_switches_to_the_copied_side_and_emits_sound_automation() {
        let params = Arc::new(PumpParams::new());
        params.set_mix(0.2);
        let queue = Arc::new(PumpAutomationQueue::default());
        let mut state = RadiantEditorState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(ClapHostParamEditSink {
                queue: Arc::clone(&queue),
                requester: None,
            }),
        );
        state.selected_curve_nodes.push(1);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::CopyAndSelectSound(SoundSide::B),
        );

        assert_eq!(params.active_sound(), SoundSide::B);
        assert!(!params.sound_sides_differ());
        assert!(state.selected_curve_nodes.is_empty());
        assert_eq!(state.undo_history.len(), 1);

        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        assert_eq!(
            queue.drain_to_output(&mut output, &mut scratch).attempted,
            3
        );
        let value = (0..buffer.len()).find_map(|index| {
            match buffer.get(index as u32)?.as_core_event()? {
                CoreEventSpace::ParamValue(value) => Some((value.param_id(), value.value())),
                _ => None,
            }
        });
        assert_eq!(value, Some((Some(PARAM_SOUND_ID), 1.0)));

        reduce_editor_message(&mut state, RadiantEditorMessage::Undo);
        assert_eq!(params.active_sound(), SoundSide::A);
        assert!(params.sound_sides_differ());
        reduce_editor_message(&mut state, RadiantEditorMessage::Redo);
        assert_eq!(params.active_sound(), SoundSide::B);
        assert!(!params.sound_sides_differ());
    }

    #[test]
    fn ab_command_copy_switches_even_when_sides_are_equal() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::CopyAndSelectSound(SoundSide::B),
        );

        assert_eq!(params.active_sound(), SoundSide::B);
        assert!(!params.sound_sides_differ());
        assert_eq!(state.undo_history.len(), 1);
        reduce_editor_message(&mut state, RadiantEditorMessage::Undo);
        assert_eq!(params.active_sound(), SoundSide::A);
        reduce_editor_message(&mut state, RadiantEditorMessage::Redo);
        assert_eq!(params.active_sound(), SoundSide::B);
    }

    #[test]
    fn equivalent_ab_copy_preserves_undo_and_redo_history() {
        let params = Arc::new(PumpParams::new());
        params.set_mix(0.2);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::SelectSound {
                side: SoundSide::B,
                copy: true,
            },
        );
        params.set_mix(0.3);
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::SelectSound {
                side: SoundSide::B,
                copy: true,
            },
        );
        reduce_editor_message(&mut state, RadiantEditorMessage::Undo);
        params.set_mix(0.2);
        assert!(!params.sound_sides_differ());

        let undo_len = state.undo_history.len();
        let redo_len = state.redo_history.len();
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::SelectSound {
                side: SoundSide::B,
                copy: true,
            },
        );

        assert_eq!(state.undo_history.len(), undo_len);
        assert_eq!(state.redo_history.len(), redo_len);
    }

    #[test]
    fn ab_buttons_activate_on_release_and_preserve_modifier_intent() {
        let bounds = Rect::from_xy_size(0.0, 0.0, 40.0, BUILD_LABEL_HEIGHT);
        let point = Point::new(20.0, BUILD_LABEL_HEIGHT * 0.5);
        let press = |modifiers| WidgetInput::PointerPress {
            position: point,
            button: PointerButton::Primary,
            modifiers,
        };
        let release = |modifiers| WidgetInput::PointerRelease {
            position: point,
            button: PointerButton::Primary,
            modifiers,
        };

        let mut switch = SoundSwitchButtonWidget::new(SoundSide::A);
        assert!(switch
            .handle_input(bounds, press(PointerModifiers::default()))
            .is_none());
        assert_eq!(
            switch
                .handle_input(bounds, release(PointerModifiers::default()))
                .and_then(|output| output.typed_cloned()),
            Some(RadiantEditorMessage::SelectSound {
                side: SoundSide::B,
                copy: false,
            })
        );

        let mut copy_switch = SoundSwitchButtonWidget::new(SoundSide::A);
        assert!(copy_switch
            .handle_input(
                bounds,
                press(PointerModifiers {
                    command: true,
                    ..PointerModifiers::default()
                }),
            )
            .is_none());
        assert_eq!(
            copy_switch
                .handle_input(bounds, release(PointerModifiers::default()))
                .and_then(|output| output.typed_cloned()),
            Some(RadiantEditorMessage::CopyAndSelectSound(SoundSide::B))
        );

        let mut option_switch = SoundSwitchButtonWidget::new(SoundSide::A);
        assert!(option_switch
            .handle_input(
                bounds,
                press(PointerModifiers {
                    alt: true,
                    ..PointerModifiers::default()
                }),
            )
            .is_none());
        assert_eq!(
            option_switch
                .handle_input(bounds, release(PointerModifiers::default()))
                .and_then(|output| output.typed_cloned()),
            Some(RadiantEditorMessage::SelectSound {
                side: SoundSide::B,
                copy: false,
            })
        );

        let mut active = SoundSideButtonWidget::new(SoundSide::A, true);
        assert!(active
            .handle_input(
                bounds,
                press(PointerModifiers {
                    command: true,
                    ..PointerModifiers::default()
                }),
            )
            .is_none());
        assert_eq!(
            active
                .handle_input(bounds, release(PointerModifiers::default()))
                .and_then(|output| output.typed_cloned()),
            Some(RadiantEditorMessage::SelectSound {
                side: SoundSide::A,
                copy: false,
            })
        );

        let mut inactive = SoundSideButtonWidget::new(SoundSide::B, false);
        assert!(inactive
            .handle_input(
                bounds,
                press(PointerModifiers {
                    command: true,
                    ..PointerModifiers::default()
                }),
            )
            .is_none());
        assert_eq!(
            inactive
                .handle_input(bounds, release(PointerModifiers::default()))
                .and_then(|output| output.typed_cloned()),
            Some(RadiantEditorMessage::SelectSound {
                side: SoundSide::B,
                copy: false,
            })
        );

        let mut option_inactive = SoundSideButtonWidget::new(SoundSide::B, false);
        assert!(option_inactive
            .handle_input(
                bounds,
                press(PointerModifiers {
                    alt: true,
                    ..PointerModifiers::default()
                }),
            )
            .is_none());
        assert_eq!(
            option_inactive
                .handle_input(bounds, release(PointerModifiers::default()))
                .and_then(|output| output.typed_cloned()),
            Some(RadiantEditorMessage::SelectSound {
                side: SoundSide::B,
                copy: true,
            })
        );
    }

    #[test]
    fn active_option_click_does_not_copy_or_create_undo_history() {
        let bounds = Rect::from_xy_size(0.0, 0.0, 40.0, BUILD_LABEL_HEIGHT);
        let point = Point::new(20.0, BUILD_LABEL_HEIGHT * 0.5);
        let mut active = SoundSideButtonWidget::new(SoundSide::A, true);
        let press = WidgetInput::PointerPress {
            position: point,
            button: PointerButton::Primary,
            modifiers: PointerModifiers {
                alt: true,
                ..PointerModifiers::default()
            },
        };
        let release = WidgetInput::PointerRelease {
            position: point,
            button: PointerButton::Primary,
            modifiers: PointerModifiers::default(),
        };

        assert!(active.handle_input(bounds, press).is_none());
        let message = active
            .handle_input(bounds, release)
            .and_then(|output| output.typed_cloned());
        assert_eq!(
            message,
            Some(RadiantEditorMessage::SelectSound {
                side: SoundSide::A,
                copy: false,
            })
        );

        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(params);
        reduce_editor_message(
            &mut state,
            message.expect("active click should select normally"),
        );
        assert!(state.undo_history.is_empty());
    }

    #[test]
    fn header_letter_buttons_lower_labels_and_show_visible_hover_without_changing_precedence() {
        let theme = pump_theme();
        let layout = LayoutOutput::default();
        let bounds = Rect::from_xy_size(0.0, 0.0, 28.9, TIMING_CONTROL_HEIGHT);
        let fill = |primitives: &[PaintPrimitive]| {
            primitives.iter().find_map(|primitive| match primitive {
                PaintPrimitive::FillRect(fill) => Some(fill.color),
                _ => None,
            })
        };
        let text_rect = |primitives: &[PaintPrimitive]| {
            primitives.iter().find_map(|primitive| match primitive {
                PaintPrimitive::Text(text) => Some(text.rect),
                _ => None,
            })
        };
        let text_baseline = |primitives: &[PaintPrimitive]| {
            primitives.iter().find_map(|primitive| match primitive {
                PaintPrimitive::Text(text) => text.baseline,
                _ => None,
            })
        };

        let mut normal = SoundSideButtonWidget::new(SoundSide::A, false);
        assert!(normal.accepts_pointer_move());
        let mut normal_primitives = Vec::new();
        normal.append_paint(&mut normal_primitives, bounds, &layout, &theme);
        assert_eq!(
            text_rect(&normal_primitives),
            Some(header_button_text_rect(bounds))
        );
        assert_eq!(
            text_baseline(&normal_primitives),
            Some(header_button_text_baseline(
                header_button_text_rect(bounds),
                PUMP_TYPOGRAPHY.body.0,
            ))
        );
        assert_eq!(fill(&normal_primitives), Some(theme.surface_base));

        normal.handle_input(
            bounds,
            WidgetInput::PointerMove {
                position: Point::new(8.0, 8.0),
            },
        );
        let mut hovered_primitives = Vec::new();
        normal.append_paint(&mut hovered_primitives, bounds, &layout, &theme);
        assert!(normal.common().state.hovered);
        assert_eq!(
            fill(&hovered_primitives),
            Some(header_button_hover_fill(&theme))
        );
        assert_ne!(fill(&hovered_primitives), fill(&normal_primitives));
        assert_eq!(
            hovered_primitives
                .iter()
                .find_map(|primitive| match primitive {
                    PaintPrimitive::FillRect(fill) => Some(fill.rect),
                    _ => None,
                }),
            Some(bounds),
            "hover must not alter the header button hit bounds"
        );

        let mut selected = SoundSideButtonWidget::new(SoundSide::B, true);
        selected.handle_input(
            bounds,
            WidgetInput::PointerMove {
                position: Point::new(8.0, 8.0),
            },
        );
        let mut selected_primitives = Vec::new();
        selected.append_paint(&mut selected_primitives, bounds, &layout, &theme);
        assert_eq!(
            fill(&selected_primitives),
            Some(theme.surface_raised.with_alpha(224)),
            "active fill keeps precedence over hover"
        );

        let help_bounds = Rect::from_xy_size(0.0, 0.0, 28.0, TIMING_CONTROL_HEIGHT);
        let mut help = HotkeyHelpButtonWidget::new();
        assert!(help.accepts_pointer_move());
        help.handle_input(
            help_bounds,
            WidgetInput::PointerMove {
                position: Point::new(8.0, 8.0),
            },
        );
        let mut help_primitives = Vec::new();
        help.append_paint(&mut help_primitives, help_bounds, &layout, &theme);
        assert!(help.common().state.hovered);
        assert_eq!(
            fill(&help_primitives),
            Some(header_button_hover_fill(&theme))
        );
        assert_eq!(
            text_rect(&help_primitives),
            Some(header_button_text_rect(help_bounds))
        );
        assert_eq!(
            text_baseline(&help_primitives),
            Some(header_button_text_baseline(
                header_button_text_rect(help_bounds),
                PUMP_TYPOGRAPHY.body.0,
            ))
        );
        help.handle_input(help_bounds, WidgetInput::FocusChanged(true));
        let mut focused_primitives = Vec::new();
        help.append_paint(&mut focused_primitives, help_bounds, &layout, &theme);
        assert!(focused_primitives.iter().any(|primitive| matches!(
            primitive,
            PaintPrimitive::StrokeRect(stroke) if stroke.color == theme.accent_warning
        )));
    }

    #[test]
    fn vim_undo_redo_shortcuts_follow_history_availability() {
        let params = Arc::new(PumpParams::new());
        let mut editor = RadiantPumpEditor::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(PumpAutomationQueue::default()),
            None,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );

        assert!(!editor.dispatch_character('u'));
        assert!(!editor.dispatch_character('U'));

        let initial_swing = params.swing();
        editor.runtime.dispatch_message(RadiantEditorMessage::Knob {
            target: NumericEntryTarget::Swing,
            message: KnobMessage::GestureStarted {
                value: initial_swing,
            },
        });
        editor.runtime.dispatch_message(RadiantEditorMessage::Knob {
            target: NumericEntryTarget::Swing,
            message: KnobMessage::ValueChanged { value: 0.25 },
        });
        editor.runtime.dispatch_message(RadiantEditorMessage::Knob {
            target: NumericEntryTarget::Swing,
            message: KnobMessage::GestureEnded { value: 0.25 },
        });
        assert_ne!(params.swing(), initial_swing);
        assert!(editor.dispatch_character('u'));
        assert_eq!(params.swing(), initial_swing);
        assert!(editor.runtime.bridge().state().undo_history.is_empty());
        assert_eq!(editor.runtime.bridge().state().redo_history.len(), 1);

        assert!(!editor.dispatch_character('u'));
        assert!(editor.dispatch_character('U'));
        assert_eq!(params.swing(), 0.25);
        assert_eq!(editor.runtime.bridge().state().undo_history.len(), 1);
        assert!(editor.runtime.bridge().state().redo_history.is_empty());
    }

    #[test]
    fn vim_shortcuts_do_not_intercept_numeric_entry() {
        let params = Arc::new(PumpParams::new());
        let mut editor = RadiantPumpEditor::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(PumpAutomationQueue::default()),
            None,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );
        let initial_swing = params.swing();
        editor.runtime.dispatch_message(RadiantEditorMessage::Knob {
            target: NumericEntryTarget::Swing,
            message: KnobMessage::GestureStarted {
                value: initial_swing,
            },
        });
        editor.runtime.dispatch_message(RadiantEditorMessage::Knob {
            target: NumericEntryTarget::Swing,
            message: KnobMessage::ValueChanged { value: 0.25 },
        });
        editor.runtime.dispatch_message(RadiantEditorMessage::Knob {
            target: NumericEntryTarget::Swing,
            message: KnobMessage::GestureEnded { value: 0.25 },
        });

        assert_eq!(editor.runtime.bridge().state().undo_history.len(), 1);
        editor
            .runtime
            .dispatch_message(RadiantEditorMessage::NumericEntry(
                NumericEntryMessage::Begin {
                    target: NumericEntryTarget::Swing,
                },
            ));
        assert!(editor.runtime.bridge().state().numeric_entry.is_some());
        assert!(!editor.dispatch_character('u'));
        assert_eq!(params.swing(), 0.25);
        assert_eq!(editor.runtime.bridge().state().undo_history.len(), 1);
    }

    #[test]
    fn hotkey_help_button_is_accessible_and_activates() {
        let bounds = Rect::from_xy_size(0.0, 0.0, PUMP_VISUAL_METRICS.icon_hit, BUILD_LABEL_HEIGHT);
        let mut widget = HotkeyHelpButtonWidget::new();
        let semantics = widget.automation_semantics();
        assert_eq!(semantics.role, AutomationRole::Button);
        assert_eq!(semantics.label.as_deref(), Some("Show hotkeys"));
        assert_eq!(
            semantics.description.as_deref(),
            Some("Open the Pump hotkey reference")
        );
        assert!(semantics.focusable);

        let mut primitives = Vec::new();
        widget.append_paint(
            &mut primitives,
            bounds,
            &LayoutOutput::default(),
            &ThemeTokens::default(),
        );
        assert!(primitives.iter().any(|primitive| matches!(
            primitive,
            PaintPrimitive::Text(text) if text.text.as_str() == "?"
        )));

        widget.handle_input(bounds, WidgetInput::FocusChanged(true));
        for key in [WidgetKey::Enter, WidgetKey::Space] {
            let output = widget
                .handle_input(bounds, WidgetInput::KeyPress(key))
                .unwrap_or_else(|| panic!("help button should activate from {key:?}"));
            assert_eq!(
                output.typed_copied::<ButtonMessage>(),
                Some(ButtonMessage::Activate)
            );
        }
    }

    #[test]
    fn hotkey_help_message_toggles_overlay_state() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(params);
        assert!(!state.hotkey_help_open);

        reduce_editor_message(&mut state, RadiantEditorMessage::ToggleHotkeyHelp);
        assert!(state.hotkey_help_open);
        reduce_editor_message(&mut state, RadiantEditorMessage::ToggleHotkeyHelp);
        assert!(!state.hotkey_help_open);
    }

    #[test]
    fn hotkey_help_activation_does_not_press_copy_action() {
        fn find_widget<'a>(
            node: &'a radiant::runtime::DevtoolsNodeSnapshot,
            label: &str,
        ) -> Option<&'a radiant::runtime::DevtoolsNodeSnapshot> {
            if node
                .widget
                .as_ref()
                .and_then(|widget| widget.semantics.label.as_deref())
                == Some(label)
            {
                return Some(node);
            }
            node.children
                .iter()
                .find_map(|child| find_widget(child, label))
        }

        let params = Arc::new(PumpParams::new());
        let mut editor = RadiantPumpEditor::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(PumpAutomationQueue::default()),
            None,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );
        let initial_snapshot = editor.runtime.devtools_snapshot();
        let help = find_widget(&initial_snapshot.root, "Show hotkeys")
            .expect("help button should be projected");
        let sound_b = find_widget(&initial_snapshot.root, "Sound B")
            .expect("sound B action should be projected");
        assert_ne!(help.node_id, sound_b.node_id);
        let bounds = help.bounds.expect("help button should have bounds");
        let center = Point::new(
            (bounds.min.x + bounds.max.x) * 0.5,
            (bounds.min.y + bounds.max.y) * 0.5,
        );
        editor.dispatch_event(radiant::runtime::Event::pointer_press(
            center,
            PointerButton::Primary,
            PointerModifiers::default(),
        ));
        editor.dispatch_event(radiant::runtime::Event::pointer_release(
            center,
            PointerButton::Primary,
            PointerModifiers::default(),
        ));

        assert!(editor.runtime.bridge().state().hotkey_help_open);
        let open_snapshot = editor.runtime.devtools_snapshot();
        let open_help = find_widget(&open_snapshot.root, "Show hotkeys")
            .expect("help button should remain projected");
        assert_eq!(open_help.node_id, help.node_id);
        let copy = find_widget(&open_snapshot.root, "Sound B")
            .expect("sound B action should be projected");
        let state = copy
            .widget
            .as_ref()
            .expect("copy action should expose widget state")
            .state;
        assert!(
            !state.pressed,
            "copy action must not inherit help button press"
        );
        assert!(
            !state.hovered,
            "copy action must not inherit help button hover"
        );
        assert!(
            !state.focused,
            "copy action must not inherit help button focus"
        );
    }

    #[test]
    fn hotkey_help_widget_exports_nonfocusable_text_semantics() {
        let widget = HotkeyHelpWidget::new();
        let semantics = widget.automation_semantics();
        assert_eq!(semantics.role, AutomationRole::Text);
        assert_eq!(semantics.label.as_deref(), Some("Pump hotkeys"));
        assert!(!semantics.focusable);

        let capabilities = widget.capabilities();
        assert!(capabilities.has_semantics());
        assert_eq!(
            capabilities
                .semantics
                .expect("hotkey helper should export semantics")
                .automation_role(),
            AutomationRole::Text
        );
    }

    #[test]
    fn action_icon_buttons_paint_svg_and_expose_button_labels_and_keyboard_activation() {
        let theme = ThemeTokens::default();
        let layout = LayoutOutput::default();
        for (icon, label, width, height) in [
            (IconName::ChevronLeft, "Undo", 64.0, CONTROL_ROW_HEIGHT),
            (IconName::ChevronRight, "Redo", 64.0, CONTROL_ROW_HEIGHT),
        ] {
            let bounds = Rect::from_xy_size(0.0, 0.0, width, height);
            let mut widget =
                ActionIconButtonWidget::new_with_state(icon, label, width, height, false);
            let semantics = widget.automation_semantics();
            assert_eq!(semantics.role, AutomationRole::Button);
            assert_eq!(semantics.label.as_deref(), Some(label));
            assert!(semantics.focusable);

            let mut primitives = Vec::new();
            widget.append_paint(&mut primitives, bounds, &layout, &theme);
            assert_eq!(
                primitives
                    .iter()
                    .filter(|primitive| matches!(primitive, PaintPrimitive::Svg(_)))
                    .count(),
                1,
                "{label} must paint one retained SVG"
            );
            assert!(
                primitives
                    .iter()
                    .all(|primitive| !matches!(primitive, PaintPrimitive::Text(_))),
                "{label} must not paint an action text glyph"
            );

            widget.handle_input(bounds, WidgetInput::FocusChanged(true));
            for key in [WidgetKey::Enter, WidgetKey::Space] {
                let output = widget
                    .handle_input(bounds, WidgetInput::KeyPress(key))
                    .unwrap_or_else(|| panic!("{label} should activate from {key:?}"));
                assert_eq!(
                    output.typed_copied::<ButtonMessage>(),
                    Some(ButtonMessage::Activate)
                );
            }
        }
    }

    #[test]
    fn settings_action_is_disabled_and_non_activatable_but_iconic() {
        let theme = ThemeTokens::default();
        let layout = LayoutOutput::default();
        let bounds = Rect::from_xy_size(0.0, 0.0, PUMP_VISUAL_METRICS.icon_hit, BUILD_LABEL_HEIGHT);
        let mut widget = ActionIconButtonWidget::new_with_state(
            IconName::Settings,
            "Settings",
            bounds.width(),
            bounds.height(),
            true,
        );
        let semantics = widget.automation_semantics();
        assert!(!semantics.focusable);
        assert!(semantics.disabled);
        assert!(!widget.common().state.disabled);
        widget.handle_input(bounds, WidgetInput::FocusChanged(true));
        assert!(widget
            .handle_input(bounds, WidgetInput::KeyPress(WidgetKey::Enter))
            .is_none());
        widget.handle_input(
            bounds,
            WidgetInput::PointerMove {
                position: Point::new(2.0, 2.0),
            },
        );
        assert!(widget.common().state.hovered);

        let mut primitives = Vec::new();
        widget.append_paint(&mut primitives, bounds, &layout, &theme);
        assert!(primitives
            .iter()
            .any(|primitive| matches!(primitive, PaintPrimitive::Svg(_))));
    }

    #[test]
    fn bypass_control_supports_pointer_keyboard_and_explicit_semantics() {
        let bounds = Rect::from_xy_size(0.0, 0.0, BYPASS_CONTROL_WIDTH, CONTROL_ROW_HEIGHT);
        let mut active = BypassControlWidget::new(false, false);
        assert_eq!(active.automation_label().as_deref(), Some("Bypass"));
        assert_eq!(active.automation_value_text().as_deref(), Some("ACTIVE"));
        assert_eq!(active.automation_checked(), Some(false));

        assert!(active
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: Point::new(8.0, 8.0),
                    button: PointerButton::Primary,
                    modifiers: Default::default(),
                },
            )
            .is_none());
        assert!(active
            .handle_input(
                bounds,
                WidgetInput::PointerRelease {
                    position: Point::new(8.0, 8.0),
                    button: PointerButton::Primary,
                    modifiers: Default::default(),
                },
            )
            .is_some());

        for key in [WidgetKey::Enter, WidgetKey::Space] {
            let mut keyboard = BypassControlWidget::new(false, false);
            let _ = keyboard.handle_input(bounds, WidgetInput::FocusChanged(true));
            assert!(keyboard
                .handle_input(bounds, WidgetInput::KeyPress(key))
                .is_some());
        }

        let bypassed = BypassControlWidget::new(true, true);
        assert_eq!(
            bypassed.automation_value_text().as_deref(),
            Some("BYPASSED")
        );
        assert_eq!(bypassed.automation_checked(), Some(true));
        assert!(bypassed.common().state.selected);
        assert!(bypassed.common().state.automation_active);
    }

    #[test]
    fn bypass_control_paints_non_color_selected_and_automation_cues_without_focus_chrome() {
        let bounds = Rect::from_xy_size(0.0, 0.0, BYPASS_CONTROL_WIDTH, CONTROL_ROW_HEIGHT);
        let mut widget = BypassControlWidget::new(true, true);
        widget.common_mut().state.focused = true;
        let mut primitives = Vec::new();
        widget.append_paint(
            &mut primitives,
            bounds,
            &LayoutOutput::default(),
            &pump_theme(),
        );

        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::Text(text) if text.text.as_str() == "BYPASSED"
            )
        }));
        let vertical_markers = primitives
            .iter()
            .filter(|primitive| {
                matches!(
                    primitive,
                    PaintPrimitive::StrokePolyline(marker)
                        if marker.points.len() == 2
                            && (marker.points[0].x - marker.points[1].x).abs() < f32::EPSILON
                )
            })
            .count();
        assert_eq!(
            vertical_markers, 2,
            "selected and automation states need independent structural markers"
        );
        assert_eq!(
            primitives
                .iter()
                .filter(|primitive| matches!(primitive, PaintPrimitive::StrokeRect(_)))
                .count(),
            1,
            "focus must not add a yellow structural outline"
        );
    }

    #[test]
    fn curve_slot_widget_click_loads_and_command_click_stores() {
        let bounds = Rect::from_xy_size(0.0, 0.0, 48.0, CURVE_SLOT_ROW_HEIGHT);
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurveSlotWidget::new(3, Some(curve), false, false);

        let load = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: Point::new(10.0, 10.0),
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers::default(),
                },
            )
            .expect("normal slot click should emit load");
        assert_eq!(
            load.typed_copied(),
            Some(CurveSlotMessage::Load { index: 3 })
        );

        let store = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: Point::new(10.0, 10.0),
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        command: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .expect("command slot click should emit store");
        assert_eq!(
            store.typed_copied(),
            Some(CurveSlotMessage::Store { index: 3 })
        );
        widget.handle_input(
            bounds,
            WidgetInput::PointerRelease {
                position: Point::new(10.0, 10.0),
                button: PointerButton::Primary,
                modifiers: PointerModifiers::default(),
            },
        );
        assert!(!widget.common.state.pressed);
    }

    #[test]
    fn curve_slot_widget_does_not_navigate_with_wheel_or_arrows() {
        let bounds = Rect::from_xy_size(0.0, 0.0, 48.0, CURVE_SLOT_ROW_HEIGHT);
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurveSlotWidget::new(3, Some(curve), false, false);

        let wheel = widget.handle_input(
            bounds,
            WidgetInput::Wheel {
                position: Point::new(10.0, 10.0),
                delta: Vector2::new(0.0, -1.0),
                modifiers: PointerModifiers::default(),
            },
        );
        assert!(wheel.is_none());

        widget.handle_input(bounds, WidgetInput::FocusChanged(true));
    }

    #[test]
    fn curve_slot_widget_paints_deviation_in_danger_color() {
        let bounds = Rect::from_xy_size(0.0, 0.0, 48.0, CURVE_SLOT_ROW_HEIGHT);
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurveSlotWidget::new(0, Some(curve), true, true);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::FillPath(path)
                    if path.brush == PaintBrush::solid(theme.accent_danger)
            )
        }));
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::FillPath(path)
                    if path.brush == PaintBrush::solid(theme.accent_danger)
                        && path.fill_rule == PaintFillRule::EvenOdd
            )
        }));
        assert!(!primitives
            .iter()
            .any(|primitive| matches!(primitive, PaintPrimitive::StrokeRect(_))));
    }

    #[test]
    fn curve_slot_widget_uses_inset_preview_and_rounded_card_primitives() {
        assert_eq!(CURVE_SLOT_ROW_HEIGHT, 40.8);
        let bounds = Rect::from_xy_size(0.0, 0.0, 48.0, CURVE_SLOT_ROW_HEIGHT);
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurveSlotWidget::new(0, Some(curve), true, false);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let preview = primitives
            .iter()
            .find_map(|primitive| match primitive {
                PaintPrimitive::StrokePolyline(polyline) if polyline.points.len() > 2 => {
                    Some(polyline)
                }
                _ => None,
            })
            .expect("curve slot should paint a sampled preview");
        assert!(preview.points.iter().all(|point| {
            point.x >= bounds.min.x + 6.8
                && point.x <= bounds.max.x - 6.8
                && point.y >= bounds.min.y + 6.8
                && point.y <= bounds.max.y - 6.8
        }));
        assert!(
            primitives
                .iter()
                .filter(|primitive| {
                    matches!(
                        primitive,
                        PaintPrimitive::FillPath(path)
                            if path.fill_rule == PaintFillRule::EvenOdd
                    )
                })
                .count()
                >= 2
        );
    }

    #[test]
    fn radiant_editor_reduces_slider_messages_to_params() {
        let params = Arc::new(PumpParams::new());
        let queue = Arc::new(PumpAutomationQueue::default());
        let mut state = RadiantEditorState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(ClapHostParamEditSink {
                queue: Arc::clone(&queue),
                requester: None,
            }),
        );

        for (target, value) in [
            (NumericEntryTarget::Mix, 0.25),
            (NumericEntryTarget::OutputGain, 0.5),
            (NumericEntryTarget::Smooth, 0.5),
            (NumericEntryTarget::Swing, 0.5),
            (NumericEntryTarget::FreeRate, 0.5),
        ] {
            reduce_editor_message(
                &mut state,
                RadiantEditorMessage::Knob {
                    target,
                    message: KnobMessage::Reset { value },
                },
            );
        }
        reduce_editor_message(&mut state, RadiantEditorMessage::SyncDivision(1.0));

        assert!((params.mix() - 0.25).abs() < f32::EPSILON);
        assert!((params.free_rate_hz() - 31.622_776).abs() < 1.0e-3);
        assert!((params.output_gain_db() + 6.0).abs() < f32::EPSILON);
        assert_eq!(params.sync_division(), MAX_SYNC_DIVISION as usize);

        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        let stats = queue.drain_to_output(&mut output, &mut scratch);
        assert_eq!(stats.attempted, 18);
        let value_ids: Vec<_> = (0..buffer.len())
            .filter_map(|index| match buffer.get(index as u32)?.as_core_event()? {
                CoreEventSpace::ParamValue(value) => value.param_id(),
                _ => None,
            })
            .collect();
        assert_eq!(
            value_ids,
            vec![
                PARAM_MIX_ID,
                PARAM_OUTPUT_GAIN_ID,
                PARAM_SMOOTH_ID,
                PARAM_SWING_ID,
                PARAM_FREE_RATE_ID,
                PARAM_SYNC_DIVISION_ID,
            ]
        );
    }

    #[test]
    fn bypass_reducer_emits_complete_clap_gesture_and_stays_out_of_undo_history() {
        let params = Arc::new(PumpParams::new());
        let queue = Arc::new(PumpAutomationQueue::default());
        let mut state = RadiantEditorState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(ClapHostParamEditSink {
                queue: Arc::clone(&queue),
                requester: None,
            }),
        );

        reduce_editor_message(&mut state, RadiantEditorMessage::ToggleBypass);
        assert!(params.bypassed());
        assert!(state.undo_history.is_empty());
        reduce_editor_message(&mut state, RadiantEditorMessage::Undo);
        assert!(params.bypassed(), "undo must never alter host bypass");

        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        let stats = queue.drain_to_output(&mut output, &mut scratch);
        assert_eq!(stats.attempted, 3);
        let value = (0..buffer.len()).find_map(|index| {
            match buffer.get(index as u32)?.as_core_event()? {
                CoreEventSpace::ParamValue(value) => Some((value.param_id(), value.value())),
                _ => None,
            }
        });
        assert_eq!(value, Some((Some(PARAM_BYPASS_ID), 1.0)));
    }

    #[test]
    fn bypass_reducer_keeps_state_active_when_clap_gesture_cannot_fit() {
        let params = Arc::new(PumpParams::new());
        let queue = Arc::new(PumpAutomationQueue::with_config(
            AutomationQueueConfig::new(3, AutomationDropPolicy::DropNewest),
        ));
        let config = AutomationConfig::default();
        assert_eq!(
            queue.push_value(&config, PARAM_MIX_ID, 0.25),
            toybox::clap::automation::AutomationEnqueueStatus::Enqueued
        );
        let mut state = RadiantEditorState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(ClapHostParamEditSink {
                queue: Arc::clone(&queue),
                requester: None,
            }),
        );

        reduce_editor_message(&mut state, RadiantEditorMessage::ToggleBypass);

        assert!(!params.bypassed());
        assert_eq!(state.automation_flush_count, 0);
        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::with_capacity(3);
        let stats = queue.drain_to_output(&mut output, &mut scratch);
        assert_eq!(stats.attempted, 1);
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn hosted_editor_reprojects_host_bypass_and_recent_automation_state() {
        let params = Arc::new(PumpParams::new());
        let mut editor = RadiantPumpEditor::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(PumpAutomationQueue::default()),
            None,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );
        let active_plan = editor.paint_plan().clone();
        assert!(active_plan.primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::Text(text) if text.text.as_str() == "ACTIVE"
            )
        }));

        crate::params::apply_param_event(params.as_ref(), PARAM_BYPASS_ID, 1.0);
        let bypassed_plan = editor.paint_plan();
        assert!(bypassed_plan.primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::Text(text) if text.text.as_str() == "BYPASSED"
            )
        }));
        assert!(bypassed_plan.primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokePolyline(marker)
                    if marker.points.len() == 2
                        && (marker.points[0].x - marker.points[1].x).abs() < f32::EPSILON
                        && (marker.width - 1.7).abs() < 1.0e-6
            )
        }));
    }

    #[test]
    fn swing_knob_transaction_updates_param_and_emits_complete_gesture() {
        let params = Arc::new(PumpParams::new());
        let queue = Arc::new(PumpAutomationQueue::default());
        let mut state = RadiantEditorState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(ClapHostParamEditSink {
                queue: Arc::clone(&queue),
                requester: None,
            }),
        );

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Knob {
                target: NumericEntryTarget::Swing,
                message: KnobMessage::GestureStarted { value: 0.0 },
            },
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Knob {
                target: NumericEntryTarget::Swing,
                message: KnobMessage::ValueChanged { value: 0.5 },
            },
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Knob {
                target: NumericEntryTarget::Swing,
                message: KnobMessage::GestureEnded { value: 0.5 },
            },
        );

        assert!((params.swing() - 0.5).abs() < f32::EPSILON);
        assert_eq!(state.undo_history.len(), 1);
        assert!(state.active_knob_gesture.is_none());

        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        assert_eq!(
            queue.drain_to_output(&mut output, &mut scratch).attempted,
            3
        );
        for index in 0..buffer.len() {
            let event = buffer
                .get(index as u32)
                .expect("queued swing gesture event should be readable");
            if let Some(CoreEventSpace::ParamValue(event)) = event.as_core_event() {
                assert_eq!(event.param_id(), Some(PARAM_SWING_ID));
                assert!((event.value() - 0.5).abs() < f64::EPSILON);
            }
        }
    }

    #[test]
    fn wheel_gesture_updates_mapped_param_with_one_ordered_transaction() {
        let params = Arc::new(PumpParams::new());
        let queue = Arc::new(PumpAutomationQueue::default());
        let mut state = RadiantEditorState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(ClapHostParamEditSink {
                queue: Arc::clone(&queue),
                requester: None,
            }),
        );
        let final_value = 0.75;
        let expected_plain = knob_plain_value(NumericEntryTarget::FreeRate, final_value).1;

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Knob {
                target: NumericEntryTarget::FreeRate,
                message: KnobMessage::WheelGesture(radiant::prelude::KnobWheelGesture::new(
                    0.5,
                    final_value,
                )),
            },
        );

        assert!((params.free_rate_hz() - expected_plain).abs() < 1.0e-3);
        assert_eq!(state.undo_history.len(), 1);
        assert!(state.active_knob_gesture.is_none());

        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        assert_eq!(
            queue.drain_to_output(&mut output, &mut scratch).attempted,
            3
        );
        let kinds: Vec<_> = (0..buffer.len())
            .map(|index| {
                let event = buffer
                    .get(index as u32)
                    .expect("wheel lifecycle event should be present");
                match event.header().type_id() {
                    ParamGestureBeginEvent::TYPE_ID => {
                        assert_eq!(
                            event
                                .as_event::<ParamGestureBeginEvent>()
                                .expect("wheel begin should decode")
                                .param_id(),
                            Some(PARAM_FREE_RATE_ID)
                        );
                        "begin"
                    }
                    ParamValueEvent::TYPE_ID => {
                        let CoreEventSpace::ParamValue(value) = event
                            .as_core_event()
                            .expect("wheel value should decode as a core event")
                        else {
                            unreachable!()
                        };
                        assert_eq!(value.param_id(), Some(PARAM_FREE_RATE_ID));
                        let expected_normalized =
                            normalized_from_plain_value(PARAM_FREE_RATE_ID, expected_plain as f64)
                                .expect("free-rate plain value should normalize");
                        assert!((value.value() - expected_normalized).abs() < f64::EPSILON);
                        "value"
                    }
                    ParamGestureEndEvent::TYPE_ID => {
                        assert_eq!(
                            event
                                .as_event::<ParamGestureEndEvent>()
                                .expect("wheel end should decode")
                                .param_id(),
                            Some(PARAM_FREE_RATE_ID)
                        );
                        "end"
                    }
                    _ => "other",
                }
            })
            .collect();
        assert_eq!(kinds, ["begin", "value", "end"]);
    }

    #[test]
    fn wheel_gesture_ends_after_rejected_value_without_mutating_state() {
        let params = Arc::new(PumpParams::new());
        let initial_rate = params.free_rate_hz();
        let queue = Arc::new(PumpAutomationQueue::with_config(
            AutomationQueueConfig::new(2, AutomationDropPolicy::DropNewest),
        ));
        let mut state = RadiantEditorState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(ClapHostParamEditSink {
                queue: Arc::clone(&queue),
                requester: None,
            }),
        );

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Knob {
                target: NumericEntryTarget::FreeRate,
                message: KnobMessage::WheelGesture(radiant::prelude::KnobWheelGesture::new(
                    0.5, 0.75,
                )),
            },
        );

        assert!((params.free_rate_hz() - initial_rate).abs() < f32::EPSILON);
        assert_eq!(state.undo_history.len(), 1);
        assert!(state.active_knob_gesture.is_none());
        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        assert_eq!(
            queue.drain_to_output(&mut output, &mut scratch).attempted,
            2
        );
        let kinds: Vec<_> = (0..buffer.len())
            .map(|index| {
                let event = buffer
                    .get(index as u32)
                    .expect("rejected wheel lifecycle event should be present");
                match event.header().type_id() {
                    ParamGestureBeginEvent::TYPE_ID => "begin",
                    ParamGestureEndEvent::TYPE_ID => "end",
                    _ => "other",
                }
            })
            .collect();
        assert_eq!(kinds, ["begin", "end"]);
    }

    #[test]
    fn radiant_numeric_entry_commit_valid_value_updates_param() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::Begin {
                target: NumericEntryTarget::Mix,
            }),
        );
        assert_eq!(
            state
                .numeric_entry
                .as_ref()
                .map(|entry| entry.draft.as_str()),
            Some("100%")
        );

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::DraftChanged {
                target: NumericEntryTarget::Mix,
                draft: "25".to_string(),
                dirty: true,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::Commit {
                target: NumericEntryTarget::Mix,
                draft: "25".to_string(),
            }),
        );

        assert!((params.mix() - 0.25).abs() < f32::EPSILON);
        assert!(state.numeric_entry.is_none());
    }

    #[test]
    fn radiant_numeric_entry_rejects_invalid_commit_without_corrupting_param() {
        let params = Arc::new(PumpParams::new());
        params.set_output_gain_db(-3.0);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::Begin {
                target: NumericEntryTarget::OutputGain,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::Commit {
                target: NumericEntryTarget::OutputGain,
                draft: "not a number".to_string(),
            }),
        );

        assert!((params.output_gain_db() + 3.0).abs() < f32::EPSILON);
        assert_eq!(
            state.numeric_entry.as_ref().map(|entry| entry.target),
            Some(NumericEntryTarget::OutputGain)
        );
    }

    #[test]
    fn radiant_numeric_entry_cancel_leaves_prior_value() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::Begin {
                target: NumericEntryTarget::FreeRate,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::DraftChanged {
                target: NumericEntryTarget::FreeRate,
                draft: "75".to_string(),
                dirty: true,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::NumericEntry(NumericEntryMessage::Cancel {
                target: NumericEntryTarget::FreeRate,
            }),
        );

        assert!((params.phase_offset() - 0.0).abs() < f32::EPSILON);
        assert!(state.numeric_entry.is_none());
    }

    #[test]
    fn numeric_value_label_starts_edit_only_on_command_click() {
        let bounds = Rect::from_xy_size(0.0, 0.0, CONTROL_VALUE_WIDTH, CONTROL_ROW_HEIGHT);
        let mut widget =
            NumericValueLabelWidget::new(NumericEntryTarget::Mix, "50%".to_string(), false, false);

        assert!(widget
            .handle_input(bounds, WidgetInput::primary_press(Point::new(8.0, 8.0)))
            .is_none());

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: Point::new(8.0, 8.0),
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        command: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .expect("command-click should begin numeric entry");

        assert_eq!(
            output.typed_cloned::<NumericEntryMessage>(),
            Some(NumericEntryMessage::Begin {
                target: NumericEntryTarget::Mix
            })
        );
    }

    #[test]
    fn numeric_value_label_edits_commits_and_cancels_draft() {
        let bounds = Rect::from_xy_size(0.0, 0.0, CONTROL_VALUE_WIDTH, CONTROL_ROW_HEIGHT);
        let mut widget =
            NumericValueLabelWidget::new(NumericEntryTarget::Mix, "100%".to_string(), true, false);

        let output = widget
            .handle_input(bounds, WidgetInput::Character('2'))
            .expect("first typed character should replace selected label");
        assert_eq!(
            output.typed_cloned::<NumericEntryMessage>(),
            Some(NumericEntryMessage::DraftChanged {
                target: NumericEntryTarget::Mix,
                draft: "2".to_string(),
                dirty: true,
            })
        );

        widget.text = "2".to_string();
        widget.dirty = true;
        let output = widget
            .handle_input(bounds, WidgetInput::Character('5'))
            .expect("second typed character should append");
        assert_eq!(
            output.typed_cloned::<NumericEntryMessage>(),
            Some(NumericEntryMessage::DraftChanged {
                target: NumericEntryTarget::Mix,
                draft: "25".to_string(),
                dirty: true,
            })
        );

        widget.text = "25".to_string();
        let output = widget
            .handle_input(bounds, WidgetInput::KeyPress(WidgetKey::Enter))
            .expect("enter should commit");
        assert_eq!(
            output.typed_cloned::<NumericEntryMessage>(),
            Some(NumericEntryMessage::Commit {
                target: NumericEntryTarget::Mix,
                draft: "25".to_string(),
            })
        );

        let output = widget
            .handle_input(bounds, WidgetInput::FocusChanged(false))
            .expect("focus loss should cancel active edit");
        assert_eq!(
            output.typed_cloned::<NumericEntryMessage>(),
            Some(NumericEntryMessage::Cancel {
                target: NumericEntryTarget::Mix,
            })
        );
    }

    #[test]
    fn radiant_editor_curve_drag_updates_editable_curve() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(1)),
        );
        assert_eq!(state.active_curve_node, Some(1));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.2, y: 0.25 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        let curve = params.editable_curve_snapshot();
        assert!((curve.nodes[1].x - 0.2).abs() < f32::EPSILON);
        assert!((curve.nodes[1].y - 0.25).abs() < f32::EPSILON);
        assert_eq!(state.active_curve_node, Some(1));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
                index: 1,
                node: CurveNode { x: 0.24, y: 0.3 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
                shift_held: false,
                option_held: false,
                command_held: false,
            }),
        );
        let curve = params.editable_curve_snapshot();
        assert!((curve.nodes[1].x - 0.24).abs() < f32::EPSILON);
        assert!((curve.nodes[1].y - 0.3).abs() < f32::EPSILON);
        assert_eq!(state.active_curve_node, None);
    }

    #[test]
    fn radiant_editor_marquee_selects_node_centers_without_mutating_curve() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.2 },
                CurveNode { x: 0.2, y: 0.8 },
                CurveNode { x: 0.5, y: 0.5 },
                CurveNode { x: 0.8, y: 0.2 },
                CurveNode { x: 1.0, y: 0.2 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let before = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressMarquee {
                start: CurveNode { x: 0.15, y: 0.9 },
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragMarquee {
                current: CurveNode { x: 0.6, y: 0.1 },
            }),
        );
        assert!(state.active_curve_marquee.is_some());
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseMarquee {
                current: CurveNode { x: 0.6, y: 0.1 },
            }),
        );

        assert_eq!(state.selected_curve_nodes, vec![1, 2]);
        assert_eq!(params.editable_curve_snapshot(), before);
        assert!(state.active_curve_marquee.is_none());
        assert!(state.undo_history.is_empty());
    }

    #[test]
    fn radiant_editor_shift_drag_from_start_locks_gain_through_vertical_drift() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.5, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
            ],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let origin = curve.nodes[2];
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressNode {
                index: 2,
                pointer: origin,
                shift_held: true,
                option_held: false,
                command_held: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.95, y: 0.05 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), curve.nodes.len() - 1);
        assert!(!dragged.nodes.contains(&curve.nodes[3]));
        assert!((dragged.nodes[2].x - 0.95).abs() < 1.0e-6);
        assert!((dragged.nodes[2].y - origin.y).abs() < 1.0e-6);
        assert!(state.shift_hover_held);
        assert_eq!(
            state
                .active_curve_node_drag
                .as_ref()
                .and_then(|drag| drag.horizontal_gain_anchor),
            Some(origin.y)
        );
    }

    #[test]
    fn radiant_editor_shift_mid_drag_engages_and_releases_without_gain_jump() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot().nodes[1];
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressNode {
                index: 1,
                pointer: origin,
                shift_held: false,
                option_held: false,
                command_held: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.42, y: 0.6 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        let engaged_gain = params.editable_curve_snapshot().nodes[1].y;

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: false,
                shift_held: true,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.5, y: 0.05 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        assert!((params.editable_curve_snapshot().nodes[1].y - engaged_gain).abs() < 1.0e-6);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: false,
                shift_held: false,
            }),
        );
        let released_gain = params.editable_curve_snapshot().nodes[1].y;
        assert!((released_gain - engaged_gain).abs() < 1.0e-6);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.55, y: 0.05 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        assert!((params.editable_curve_snapshot().nodes[1].y - engaged_gain).abs() < 1.0e-6);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.58, y: 0.15 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        assert!(
            (params.editable_curve_snapshot().nodes[1].y - (engaged_gain + 0.1)).abs() < 1.0e-6
        );
    }

    #[test]
    fn radiant_editor_shift_drag_preserves_wrapped_endpoint_gain() {
        let params = Arc::new(PumpParams::new());
        let curve = params.editable_curve_snapshot();
        let origin = curve.nodes[0];
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressNode {
                index: 0,
                pointer: origin,
                shift_held: true,
                option_held: false,
                command_held: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 0,
                node: CurveNode { x: 0.8, y: 0.2 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        let dragged = params.editable_curve_snapshot();
        let last = dragged.nodes.len() - 1;
        assert_eq!(dragged.nodes[0].x, 0.0);
        assert_eq!(dragged.nodes[last].x, 1.0);
        assert!((dragged.nodes[0].y - origin.y).abs() < 1.0e-6);
        assert!((dragged.nodes[last].y - origin.y).abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_shift_anchor_does_not_leak_into_consecutive_gesture() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot().nodes[1];
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressNode {
                index: 1,
                pointer: origin,
                shift_held: true,
                option_held: false,
                command_held: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
                index: 1,
                node: CurveNode { x: 0.4, y: 0.1 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
                shift_held: true,
                option_held: false,
                command_held: false,
            }),
        );
        assert!(state.active_curve_node_drag.is_none());

        let second_origin = params.editable_curve_snapshot().nodes[1];
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressNode {
                index: 1,
                pointer: second_origin,
                shift_held: false,
                option_held: false,
                command_held: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.45, y: 0.25 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        assert!((params.editable_curve_snapshot().nodes[1].y - 0.25).abs() < 1.0e-6);
        assert!(!state.shift_hover_held);
    }

    #[test]
    fn radiant_editor_shift_option_drag_from_start_locks_time_while_gain_moves() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.8 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.53, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 0.8 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let origin = curve.nodes[2];
        let mut state = editor_state(Arc::clone(&params));

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 2,
                pointer: origin,
                shift_held: true,
                option_held: true,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.98, y: 0.05 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), curve.nodes.len());
        assert!((dragged.nodes[2].x - origin.x).abs() < 1.0e-6);
        assert!((dragged.nodes[2].y - 0.05).abs() < 1.0e-6);
        assert_eq!(
            state
                .active_curve_node_drag
                .as_ref()
                .and_then(|drag| drag.vertical_time_anchor),
            Some(origin.x)
        );
        assert!(state
            .active_curve_node_drag
            .as_ref()
            .is_some_and(|drag| drag.horizontal_gain_anchor.is_none()));
    }

    #[test]
    fn radiant_editor_shift_option_mid_drag_engages_and_releases_without_jump() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot().nodes[1];
        let mut state = editor_state(Arc::clone(&params));

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 1,
                pointer: origin,
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.42, y: 0.6 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let engaged = params.editable_curve_snapshot().nodes[1];

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: true,
                command_held: false,
                shift_held: true,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.9, y: 0.2 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let constrained = params.editable_curve_snapshot().nodes[1];
        assert!((constrained.x - engaged.x).abs() < 1.0e-6);
        assert!((constrained.y - 0.2).abs() < 1.0e-6);

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: false,
                shift_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.9, y: 0.2 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let released = params.editable_curve_snapshot().nodes[1];
        assert!((released.x - constrained.x).abs() < 1.0e-6);
        assert!((released.y - constrained.y).abs() < 1.0e-6);

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.94, y: 0.3 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let resumed = params.editable_curve_snapshot().nodes[1];
        assert!((resumed.x - (released.x + 0.04)).abs() < 1.0e-6);
        assert!((resumed.y - 0.3).abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_option_release_transitions_smoothly_to_shift_only() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.8 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.53, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 0.8 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let origin = curve.nodes[2];
        let mut state = editor_state(Arc::clone(&params));

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 2,
                pointer: origin,
                shift_held: true,
                option_held: true,
                command_held: true,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.9, y: 0.1 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let vertical = params.editable_curve_snapshot().nodes[2];
        assert!((vertical.x - origin.x).abs() < 1.0e-6);
        assert!((vertical.y - 0.1).abs() < 1.0e-6);

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: true,
                shift_held: true,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.9, y: 0.1 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let handoff = params.editable_curve_snapshot().nodes[2];
        assert!((handoff.x - vertical.x).abs() < 1.0e-6);
        assert!((handoff.y - vertical.y).abs() < 1.0e-6);

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: false,
                shift_held: true,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.94, y: 0.9 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let horizontal = params.editable_curve_snapshot().nodes[2];
        assert!((horizontal.x - (handoff.x + 0.04)).abs() < 1.0e-6);
        assert!((horizontal.y - handoff.y).abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_shift_option_precedes_command_and_preserves_boundaries() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.8 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.53, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 0.8 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let origin = curve.nodes[2];
        let mut state = editor_state(Arc::clone(&params));

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 2,
                pointer: origin,
                shift_held: true,
                option_held: true,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: true,
                command_held: true,
                shift_held: true,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 1.5, y: 1.5 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let high = params.editable_curve_snapshot();
        assert_eq!(high.nodes.len(), curve.nodes.len());
        assert!((high.nodes[2].x - origin.x).abs() < 1.0e-6);
        assert!((high.nodes[2].y - 1.0).abs() < 1.0e-6);

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: true,
                command_held: false,
                shift_held: true,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: -0.5, y: -0.5 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let low = params.editable_curve_snapshot();
        assert_eq!(low.nodes.len(), curve.nodes.len());
        assert!((low.nodes[2].x - origin.x).abs() < 1.0e-6);
        assert!(low.nodes[2].y.abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_vertical_anchor_clears_on_cancel_and_consecutive_gesture() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot().nodes[1];
        let mut state = editor_state(Arc::clone(&params));

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 1,
                pointer: origin,
                shift_held: true,
                option_held: true,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.8, y: 0.2 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        reduce_curve_message(&mut state, CurvePreviewMessage::Cancel);
        assert!(state.active_curve_node_drag.is_none());
        assert!(!state.shift_hover_held);
        assert!(!state.option_hover_held);

        let second_origin = params.editable_curve_snapshot().nodes[1];
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 1,
                pointer: second_origin,
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.4, y: 0.4 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );
        let second = params.editable_curve_snapshot().nodes[1];
        assert!((second.x - 0.4).abs() < 1.0e-6);
        assert!((second.y - 0.4).abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_curve_drag_sticks_before_neighbor_boundary() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.5, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
            ],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));
        let preview_width = 300.0;
        let threshold_x = curve_node_push_through_threshold_x(preview_width);
        let visible_margin_px =
            threshold_x * (curve_viewport_width(preview_width).max(1.0) - 1.0).max(1.0);
        assert!((visible_margin_px - CURVE_NODE_PUSH_THROUGH_MARGIN_PX).abs() < f32::EPSILON);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(2)),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode {
                    x: curve.nodes[3].x + threshold_x - 1.0e-3,
                    y: 0.4,
                },
                push_through_threshold_x: threshold_x,
            }),
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), curve.nodes.len());
        assert_eq!(dragged.nodes[3], curve.nodes[3]);
        assert!(dragged.nodes[2].x <= curve.nodes[3].x - CURVE_NODE_MIN_SPACING_X);
        assert_eq!(state.active_curve_node, Some(2));
    }

    #[test]
    fn radiant_editor_curve_drag_removes_crossed_neighbor() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.5, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
            ],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(2)),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.95, y: 0.4 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), curve.nodes.len() - 1);
        assert!(!dragged.nodes.contains(&curve.nodes[3]));
        assert_eq!(dragged.segments.len(), dragged.nodes.len() - 1);
        assert_eq!(state.active_curve_node, Some(2));
        assert_eq!(state.hover_curve_node, Some(2));
    }

    #[test]
    fn radiant_editor_curve_drag_removes_multiple_crossed_neighbors() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.2, y: 0.6 },
                CurveNode { x: 0.4, y: 0.3 },
                CurveNode { x: 0.6, y: 0.7 },
                CurveNode { x: 0.8, y: 0.2 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
            ],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(1)),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.98, y: 0.45 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        let dragged = params.editable_curve_snapshot();
        assert_eq!(dragged.nodes.len(), 3);
        assert_eq!(dragged.nodes[0].x, 0.0);
        assert!((dragged.nodes[1].x - 0.98).abs() < 1.0e-6);
        assert_eq!(dragged.nodes[2].x, 1.0);
        assert_eq!(state.active_curve_node, Some(1));
    }

    #[test]
    fn radiant_editor_selected_nodes_drag_as_group_with_spacing_clamp() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.8 },
                CurveNode { x: 0.2, y: 0.2 },
                CurveNode { x: 0.4, y: 0.4 },
                CurveNode { x: 0.7, y: 0.6 },
                CurveNode { x: 1.0, y: 0.8 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));
        state.selected_curve_nodes = vec![1, 2];

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 1,
                pointer: curve.nodes[1],
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );
        assert_eq!(state.selected_curve_nodes, vec![1, 2]);
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::DragNode {
                index: 1,
                node: CurveNode { x: 0.95, y: 0.8 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            },
        );

        let moved = params.editable_curve_snapshot();
        assert_eq!(moved.nodes.len(), curve.nodes.len());
        assert!((moved.nodes[1].x - (moved.nodes[2].x - 0.2)).abs() < 1.0e-6);
        assert!(moved.nodes[2].x <= moved.nodes[3].x - CURVE_NODE_MIN_SPACING_X);
        assert!((moved.nodes[1].y - 0.8).abs() < 1.0e-6);
        assert!((moved.nodes[2].y - 1.0).abs() < 1.0e-6);
        assert_eq!(moved.nodes[0].x, 0.0);
        assert_eq!(moved.nodes[4].x, 1.0);
        assert_eq!(state.selected_curve_nodes, vec![1, 2]);

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ReleaseNode {
                index: 1,
                node: moved.nodes[1],
                push_through_threshold_x: test_curve_push_through_threshold_x(),
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );
        assert_eq!(state.selected_curve_nodes, vec![1, 2]);
    }

    #[test]
    fn radiant_editor_pressing_unselected_node_clears_group_selection() {
        let params = Arc::new(PumpParams::new());
        let curve = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));
        state.selected_curve_nodes = vec![1];

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 2,
                pointer: curve.nodes[2],
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );

        assert!(state.selected_curve_nodes.is_empty());
        assert!(state
            .active_curve_node_drag
            .as_ref()
            .is_some_and(|drag| drag.selected_indices.is_empty()));
    }

    #[test]
    fn radiant_editor_delete_selected_nodes_preserves_endpoints_and_clears_state() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.5 },
                CurveNode { x: 0.3, y: 0.2 },
                CurveNode { x: 0.6, y: 0.8 },
                CurveNode { x: 1.0, y: 0.5 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 3],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));
        state.selected_curve_nodes = vec![0, 1, 2, 3];
        state.active_curve_marquee = Some(ActiveCurveMarquee {
            start: curve.nodes[1],
            current: curve.nodes[2],
        });

        reduce_curve_message(&mut state, CurvePreviewMessage::DeleteSelectedNodes);

        let remaining = params.editable_curve_snapshot();
        assert_eq!(remaining.nodes.len(), 2);
        assert_eq!(remaining.segments.len(), 1);
        assert_eq!(remaining.nodes[0].x, 0.0);
        assert_eq!(remaining.nodes[1].x, 1.0);
        assert!(state.selected_curve_nodes.is_empty());
        assert!(state.active_curve_marquee.is_none());
        assert!(state.active_curve_node_drag.is_none());
    }

    #[test]
    fn radiant_editor_curve_drag_reverse_restores_buffered_neighbors() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.5, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.15 },
                CurveSegment { tension: -0.25 },
                CurveSegment { tension: 0.35 },
                CurveSegment { tension: -0.05 },
            ],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(2)),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.95, y: 0.4 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        assert_eq!(
            params.editable_curve_snapshot().nodes.len(),
            curve.nodes.len() - 1
        );

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 2,
                node: CurveNode { x: 0.55, y: 0.4 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        let restored = params.editable_curve_snapshot();
        assert_eq!(restored.nodes.len(), curve.nodes.len());
        assert_eq!(restored.nodes[3], curve.nodes[3]);
        assert_eq!(restored.segments, curve.segments);
        assert_eq!(state.active_curve_node, Some(2));
    }

    #[test]
    fn radiant_editor_curve_drag_release_commits_visible_crossings() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.25, y: 0.6 },
                CurveNode { x: 0.5, y: 0.3 },
                CurveNode { x: 0.75, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
                CurveSegment { tension: 0.0 },
            ],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(2)),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseNode {
                index: 2,
                node: CurveNode { x: 0.95, y: 0.4 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
                shift_held: false,
                option_held: false,
                command_held: false,
            }),
        );

        let released = params.editable_curve_snapshot();
        assert_eq!(released.nodes.len(), curve.nodes.len() - 1);
        assert!(!released.nodes.contains(&curve.nodes[3]));
        assert_eq!(state.active_curve_node, None);
        assert!(state.active_curve_node_drag.is_none());
    }

    #[test]
    fn radiant_editor_curve_drag_keeps_endpoints_anchored() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.5, y: 0.25 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(unconstrained_press(0)),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: 0,
                node: CurveNode { x: 0.9, y: 0.31 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );

        let dragged = params.editable_curve_snapshot();
        let last_index = dragged.nodes.len() - 1;
        assert_eq!(dragged.nodes.len(), curve.nodes.len());
        assert_eq!(dragged.nodes[0].x, 0.0);
        assert_eq!(dragged.nodes[last_index].x, 1.0);
        assert!((dragged.nodes[0].y - 0.31).abs() < 1.0e-6);
        assert!((dragged.nodes[last_index].y - 0.31).abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_curve_delete_removes_interior_node() {
        let params = Arc::new(PumpParams::new());
        let before = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));
        state.active_curve_node = Some(1);
        state.hover_curve_node = Some(1);
        state.preview_curve_node = Some(CurveNode { x: 0.2, y: 0.3 });
        state.hover_curve_segment = Some(0);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DeleteNode { index: 1 }),
        );

        let after = params.editable_curve_snapshot();
        assert_eq!(after.nodes.len() + 1, before.nodes.len());
        assert_eq!(after.segments.len(), after.nodes.len() - 1);
        assert_eq!(state.active_curve_node, None);
        assert!(state.active_curve_segment.is_none());
        assert_eq!(state.hover_curve_node, None);
        assert_eq!(state.preview_curve_node, None);
        assert_eq!(state.hover_curve_segment, None);
    }

    #[test]
    fn radiant_editor_curve_delete_ignores_endpoints() {
        let params = Arc::new(PumpParams::new());
        let before = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));
        state.hover_curve_node = Some(0);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DeleteNode { index: 0 }),
        );

        assert_eq!(params.editable_curve_snapshot().nodes, before.nodes);
        assert_eq!(params.editable_curve_snapshot().segments, before.segments);
    }

    #[test]
    fn radiant_editor_curve_insert_adds_preview_node_to_params() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));
        state.preview_curve_node = Some(CurveNode { x: 0.2, y: 0.0 });
        state.hover_curve_segment = Some(1);
        state.option_hover_held = true;
        let before = params.editable_curve_snapshot();
        let node = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&before, 0.2),
        };

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::InsertNode {
                node,
                command_held: false,
            }),
        );

        let after = params.editable_curve_snapshot();
        assert_eq!(after.nodes.len(), before.nodes.len() + 1);
        assert_eq!(after.segments.len(), after.nodes.len() - 1);
        assert_eq!(state.active_curve_node, Some(2));
        assert_eq!(state.preview_curve_node, None);
        assert_eq!(state.hover_curve_segment, None);
        assert!(after
            .nodes
            .iter()
            .any(|inserted| (inserted.x - node.x).abs() < 1.0e-6
                && (inserted.y - node.y).abs() < 1.0e-6));
    }

    #[test]
    fn radiant_editor_option_hover_clears_insert_preview() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(params);
        state.preview_curve_node = Some(CurveNode { x: 0.2, y: 0.3 });
        state.hover_curve_segment = Some(1);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                option_held: true,
                command_held: false,
                shift_held: false,
            }),
        );

        assert_eq!(state.preview_curve_node, None);
        assert_eq!(state.hover_curve_segment, Some(1));
        assert!(state.option_hover_held);
    }

    #[test]
    fn radiant_editor_segment_drag_bends_with_pointer_direction() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.2 },
                CurveNode { x: 0.5, y: 0.8 },
                CurveNode { x: 1.0, y: 0.2 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }, CurveSegment { tension: 0.0 }],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));
        state.option_hover_held = true;
        let start = Point::new(120.0, 48.0);
        let baseline_midpoint = sample_editable_curve(&curve, 0.25);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressSegment {
                index: 0,
                position: start,
            }),
        );
        assert!(state.active_curve_segment.is_some());
        assert_eq!(state.preview_curve_node, None);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragSegment {
                index: 0,
                position: Point::new(start.x, start.y - 24.0),
                curve_size: Vector2::new(320.0, CURVE_PREVIEW_HEIGHT),
            }),
        );
        let upward_midpoint = sample_editable_curve(&params.editable_curve_snapshot(), 0.25);
        assert!(upward_midpoint > baseline_midpoint);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragSegment {
                index: 0,
                position: Point::new(start.x, start.y + 24.0),
                curve_size: Vector2::new(320.0, CURVE_PREVIEW_HEIGHT),
            }),
        );
        let downward_midpoint = sample_editable_curve(&params.editable_curve_snapshot(), 0.25);
        assert!(downward_midpoint < baseline_midpoint);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseSegment {
                index: 0,
                position: Point::new(start.x, start.y + 24.0),
                curve_size: Vector2::new(320.0, CURVE_PREVIEW_HEIGHT),
            }),
        );
        assert!(state.active_curve_segment.is_none());
    }

    #[test]
    fn radiant_editor_command_segment_drag_translates_pair_without_changing_slope() {
        let params = Arc::new(PumpParams::new());
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.2 },
                CurveNode { x: 0.25, y: 0.3 },
                CurveNode { x: 0.55, y: 0.7 },
                CurveNode { x: 0.8, y: 0.4 },
                CurveNode { x: 1.0, y: 0.2 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));
        state.command_hover_held = true;
        let start = Point::new(140.0, 40.0);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressSegmentMove {
                index: 1,
                position: start,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragSegment {
                index: 1,
                position: Point::new(start.x + 24.0, start.y - 12.0),
                curve_size: Vector2::new(320.0, CURVE_PREVIEW_HEIGHT),
            }),
        );

        let moved = params.editable_curve_snapshot();
        let left_delta = (
            moved.nodes[1].x - curve.nodes[1].x,
            moved.nodes[1].y - curve.nodes[1].y,
        );
        let right_delta = (
            moved.nodes[2].x - curve.nodes[2].x,
            moved.nodes[2].y - curve.nodes[2].y,
        );
        assert!((left_delta.0 - right_delta.0).abs() < 1.0e-6);
        assert!((left_delta.1 - right_delta.1).abs() < 1.0e-6);
        assert!(
            ((moved.nodes[2].y - moved.nodes[1].y) - (curve.nodes[2].y - curve.nodes[1].y)).abs()
                < 1.0e-6
        );
        assert!(state
            .active_curve_segment
            .as_ref()
            .is_some_and(|drag| { drag.mode == CurveSegmentDragMode::MovePair }));
    }

    #[test]
    fn radiant_editor_command_release_clears_segment_move_without_mutation() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));
        let before = params.editable_curve_snapshot();
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressSegmentMove {
                index: 1,
                position: Point::new(120.0, 40.0),
            }),
        );
        assert!(state.command_hover_held);
        assert!(state
            .active_curve_segment
            .as_ref()
            .is_some_and(|drag| drag.mode == CurveSegmentDragMode::MovePair));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: false,
                shift_held: false,
            }),
        );

        assert_eq!(params.editable_curve_snapshot(), before);
        assert!(state.active_curve_segment.is_none());
        assert_eq!(state.hover_curve_segment, None);
        assert!(!state.command_hover_held);
    }

    #[test]
    fn curve_preview_widget_emits_press_for_hit_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: Default::default(),
                },
            )
            .expect("hit node should emit a press message");

        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::PressNode {
                index: 1,
                pointer: CurvePreviewWidget::node_from_point(bounds, position),
                shift_held: false,
                option_held: false,
                command_held: false,
            })
        );
    }

    #[test]
    fn curve_preview_widget_emits_paint_for_secondary_gesture() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.42, y: 0.34 });
        let mut press_widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let sample = press_widget.paint_sample_from_display_point(bounds, position);
        assert_eq!(
            press_widget
                .handle_input(
                    bounds,
                    WidgetInput::PointerPress {
                        position,
                        button: PointerButton::Secondary,
                        modifiers: Default::default(),
                    },
                )
                .and_then(|output| output.typed_copied()),
            Some(CurvePreviewMessage::PressPaint { sample })
        );

        let mut drag_widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_active_curve_paint(true);
        let drag_position = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.58, y: 0.72 });
        assert!(matches!(
            drag_widget
                .handle_input(
                    bounds,
                    WidgetInput::PointerMove {
                        position: drag_position
                    }
                )
                .and_then(|output| output.typed_copied()),
            Some(CurvePreviewMessage::DragPaint { .. })
        ));
        assert!(matches!(
            drag_widget
                .handle_input(
                    bounds,
                    WidgetInput::PointerRelease {
                        position: drag_position,
                        button: PointerButton::Secondary,
                        modifiers: Default::default(),
                    },
                )
                .and_then(|output| output.typed_copied()),
            Some(CurvePreviewMessage::ReleasePaint { .. })
        ));
    }

    #[test]
    fn curve_preview_widget_projects_captured_paint_motion_to_nearest_boundary() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let curve_bounds = CurvePreviewWidget::curve_bounds(bounds);
        let outside_positions = [
            Point::new(curve_bounds.min.x - 1.0, curve_bounds.center().y),
            Point::new(curve_bounds.max.x + 1.0, curve_bounds.center().y),
            Point::new(curve_bounds.center().x, curve_bounds.min.y - 1.0),
            Point::new(curve_bounds.center().x, curve_bounds.max.y + 1.0),
            Point::new(curve_bounds.min.x - 1.0, curve_bounds.min.y - 1.0),
            Point::new(curve_bounds.max.x + 1.0, curve_bounds.max.y + 1.0),
        ];
        let widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        for position in outside_positions {
            let projected = Point::new(
                position.x.clamp(curve_bounds.min.x, curve_bounds.max.x),
                position.y.clamp(curve_bounds.min.y, curve_bounds.max.y),
            );
            let expected_boundary_conversion =
                widget.paint_sample_from_display_point(bounds, projected);
            let actual = widget.paint_sample_from_boundary(bounds, position);
            assert_eq!(actual.node, expected_boundary_conversion.node);
            assert_eq!(
                actual.display_position,
                CurvePreviewWidget::normalized_display_position(bounds, position)
            );
            assert!(actual.outside);

            let mut active_widget =
                CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false)
                    .with_active_curve_paint(true);
            assert_eq!(
                active_widget
                    .handle_input(bounds, WidgetInput::PointerMove { position })
                    .and_then(|output| output.typed_copied()),
                Some(CurvePreviewMessage::DragPaintOutside { sample: actual }),
                "captured outside motion must preserve the raw observation at {position:?}"
            );
            assert_eq!(
                active_widget
                    .handle_input(
                        bounds,
                        WidgetInput::PointerRelease {
                            position,
                            button: PointerButton::Secondary,
                            modifiers: Default::default(),
                        },
                    )
                    .and_then(|output| output.typed_copied()),
                Some(CurvePreviewMessage::ReleasePaintOutside { sample: actual })
            );
        }
    }

    #[test]
    fn curve_preview_widget_to_reducer_preserves_diagonal_exit_intersection() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let curve_bounds = CurvePreviewWidget::curve_bounds(bounds);
        let width = (curve_bounds.width().max(1.0) - 1.0).max(1.0);
        let height = (curve_bounds.height().max(1.0) - 1.0).max(1.0);
        let start_position =
            CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.25, y: 0.25 });
        let outside_position = Point::new(
            curve_bounds.min.x + width * 1.5,
            curve_bounds.min.y + height * (1.0 - 1.35),
        );

        let mut press_widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let start_sample = match press_widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: start_position,
                    button: PointerButton::Secondary,
                    modifiers: Default::default(),
                },
            )
            .and_then(|output| output.typed_copied())
        {
            Some(CurvePreviewMessage::PressPaint { sample }) => sample,
            other => panic!("expected paint press, got {other:?}"),
        };

        let mut drag_widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_active_curve_paint(true);
        let outside_sample = match drag_widget
            .handle_input(
                bounds,
                WidgetInput::PointerMove {
                    position: outside_position,
                },
            )
            .and_then(|output| output.typed_copied())
        {
            Some(CurvePreviewMessage::DragPaintOutside { sample }) => sample,
            other => panic!("expected outside paint drag, got {other:?}"),
        };
        assert!(outside_sample.outside);
        assert!(outside_sample.display_position.x > 1.0);
        assert!(outside_sample.display_position.y > 1.0);

        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint {
                sample: start_sample,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaintOutside {
                sample: outside_sample,
            }),
        );

        let points = state
            .active_curve_paint
            .as_ref()
            .expect("paint remains active")
            .preview_runs()[0]
            .points()
            .to_vec();
        assert_eq!(points[0].position, start_sample.raw_position());
        let t = (1.0 - start_sample.display_position.x)
            / (outside_sample.display_position.x - start_sample.display_position.x);
        let expected_exit_y = start_sample.display_position.y
            + (outside_sample.display_position.y - start_sample.display_position.y) * t;
        assert!((points[1].position.x - 1.0).abs() <= CURVE_PAINT_ASSERT_EPSILON);
        assert!((points[1].position.y - expected_exit_y).abs() <= CURVE_PAINT_ASSERT_EPSILON);
    }

    #[test]
    fn active_curve_paint_preserves_ordered_boundary_observations() {
        let origin_snapshot = editor_state(Arc::new(PumpParams::new())).snapshot();
        let mut paint = ActiveCurvePaint::new(origin_snapshot, 0.0);
        let start = paint_sample(0.35, 0.4);
        let first = boundary_paint_sample(0.0, 0.7);
        let replacement = boundary_paint_sample(0.0, 0.2);
        paint.push_sample(start);
        paint.push_boundary_sample(first);
        paint.push_boundary_sample(replacement);

        let runs = paint.preview_runs();
        assert_eq!(runs.len(), 1);
        let points = runs[0].points();
        assert_eq!(
            points.first().map(|point| point.position),
            Some(start.raw_position())
        );
        assert!(points.iter().any(|point| {
            point.position == first.raw_position()
                && matches!(
                    point.contact,
                    BoundaryContact::Edge(EdgeParameter {
                        edge: BoundaryEdge::Left,
                        ..
                    })
                )
        }));
        assert_eq!(
            points.last().map(|point| point.position),
            Some(first.raw_position())
        );
    }

    #[test]
    fn truncated_active_curve_paint_preview_and_release_use_retained_geometry() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint {
                sample: paint_sample(0.1, 0.2),
            }),
        );
        for index in 1..=128 {
            let (x, y) = if index % 2 == 0 {
                (0.1, 0.2)
            } else {
                (0.9, 0.8)
            };
            reduce_editor_message(
                &mut state,
                RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint {
                    sample: paint_sample(x, y),
                }),
            );
        }

        let paint = state
            .active_curve_paint
            .as_ref()
            .expect("truncated paint remains active until release");
        assert!(paint.recorder.is_truncated());
        assert_ne!(paint.preview_candidate(), origin);
        assert!(matches!(
            paint.finished_curve(),
            PaintCommitOutcome::Applied { .. }
        ));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint {
                sample: Some(paint_sample(0.1, 0.2)),
            }),
        );
        assert_ne!(params.editable_curve_snapshot(), origin);
        assert_eq!(state.undo_history.len(), 1);
    }

    #[test]
    fn curve_paint_reconstructor_preserves_order_and_boundary_seams() {
        let origin = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
            segments: vec![CurveSegment { tension: 0.0 }],
            ..EditableCurve::default()
        }
        .normalized();
        let run = recorded_run([
            RectPoint { x: 0.0, y: 0.4 },
            RectPoint { x: 0.25, y: 0.2 },
            RectPoint { x: 0.5, y: 0.8 },
            RectPoint { x: 0.75, y: 0.2 },
            RectPoint { x: 1.0, y: 0.6 },
        ]);

        let outcome = reconstruct_paint(&origin, 0.0, &[run]);
        let candidate = match outcome {
            PaintCommitOutcome::Applied { candidate } => candidate,
            other => panic!("boundary paint should apply, got {other:?}"),
        };
        assert_curve_paint_topology_is_bounded(&candidate);
        assert!(candidate.nodes.iter().any(|node| {
            (node.x - 0.25).abs() <= CURVE_PAINT_ASSERT_EPSILON
                && (node.y - 0.2).abs() <= CURVE_PAINT_ASSERT_EPSILON
        }));
        assert!(candidate.nodes.iter().any(|node| {
            (node.x - 0.5).abs() <= CURVE_PAINT_ASSERT_EPSILON
                && (node.y - 0.8).abs() <= CURVE_PAINT_ASSERT_EPSILON
        }));
        assert!(candidate.nodes.iter().any(|node| {
            (node.x - 0.75).abs() <= CURVE_PAINT_ASSERT_EPSILON
                && (node.y - 0.2).abs() <= CURVE_PAINT_ASSERT_EPSILON
        }));
    }

    #[test]
    fn curve_paint_prefers_one_curved_segment_over_an_overfit_sampled_stroke() {
        let origin = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
            segments: vec![CurveSegment { tension: 0.0 }],
            ..EditableCurve::default()
        }
        .normalized();
        let left = CurveNode { x: 0.1, y: 0.15 };
        let right = CurveNode { x: 0.9, y: 0.85 };
        let source_tension = 0.65;
        let run = sampled_segment_run(left, right, source_tension, 16);

        let outcome = reconstruct_paint(&origin, 0.0, &[run]);
        assert!(matches!(&outcome, PaintCommitOutcome::Applied { .. }));
        let candidate = outcome.candidate();
        let interval_nodes = candidate
            .nodes
            .iter()
            .filter(|node| {
                node.x >= left.x - CURVE_PAINT_ASSERT_EPSILON
                    && node.x <= right.x + CURVE_PAINT_ASSERT_EPSILON
            })
            .collect::<Vec<_>>();
        assert_eq!(interval_nodes.len(), 2, "candidate: {candidate:?}");
        assert!((interval_nodes[0].x - left.x).abs() <= CURVE_PAINT_ASSERT_EPSILON);
        assert!((interval_nodes[1].x - right.x).abs() <= CURVE_PAINT_ASSERT_EPSILON);

        let painted_segment = candidate
            .nodes
            .windows(2)
            .zip(candidate.segments.iter())
            .find(|(nodes, _)| {
                (nodes[0].x - left.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
                    && (nodes[1].x - right.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
            })
            .map(|(_, segment)| segment)
            .expect("painted endpoints should remain adjacent");
        assert!(painted_segment.tension.abs() > 0.2);
        assert!((painted_segment.tension - source_tension).abs() <= 0.08);
    }

    #[test]
    fn curve_paint_keeps_a_sampled_linear_stroke_near_zero_tension() {
        let origin = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
            segments: vec![CurveSegment { tension: 0.0 }],
            ..EditableCurve::default()
        }
        .normalized();
        let left = CurveNode { x: 0.1, y: 0.15 };
        let right = CurveNode { x: 0.9, y: 0.85 };
        let run = sampled_segment_run(left, right, 0.0, 16);

        let outcome = reconstruct_paint(&origin, 0.0, &[run]);
        assert!(matches!(&outcome, PaintCommitOutcome::Applied { .. }));
        let candidate = outcome.candidate();
        let interval_nodes = candidate
            .nodes
            .iter()
            .filter(|node| {
                node.x >= left.x - CURVE_PAINT_ASSERT_EPSILON
                    && node.x <= right.x + CURVE_PAINT_ASSERT_EPSILON
            })
            .collect::<Vec<_>>();
        assert_eq!(interval_nodes.len(), 2, "candidate: {candidate:?}");

        let painted_segment = candidate
            .nodes
            .windows(2)
            .zip(candidate.segments.iter())
            .find(|(nodes, _)| {
                (nodes[0].x - left.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
                    && (nodes[1].x - right.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
            })
            .map(|(_, segment)| segment)
            .expect("painted endpoints should remain adjacent");
        assert!(painted_segment.tension.abs() <= 0.05);
    }

    #[test]
    fn curve_paint_keeps_origin_until_release_and_commits_candidate_once() {
        let params = Arc::new(PumpParams::new());
        let origin = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
            segments: vec![CurveSegment { tension: 0.0 }],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&origin);
        let mut state = editor_state(Arc::clone(&params));
        let first = paint_sample(0.2, 0.2);
        let second = paint_sample(0.8, 0.8);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: first }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint { sample: second }),
        );

        let paint = state
            .active_curve_paint
            .as_ref()
            .expect("paint remains active before release");
        let preview = paint.preview_candidate();
        let release = paint.finished_curve();
        assert!(matches!(&release, PaintCommitOutcome::Applied { .. }));
        assert_eq!(preview, *release.candidate());
        assert_ne!(preview, origin);

        let runs = paint.preview_runs();
        let widget = CurvePreviewWidget::new(origin.clone(), None, None, None, None, None, false)
            .with_active_curve_paint(true)
            .with_curve_paint_runs(Some(runs));
        assert_eq!(widget.curve, origin);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();
        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokePolyline(polyline)
                    if polyline.color == theme.accent_mint
                        && (polyline.width - 1.7).abs() <= 1.0e-6
            )
        }));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
        );
        assert_eq!(params.editable_curve_snapshot(), preview);
        assert_eq!(state.undo_history.len(), 1);

        reduce_editor_message(&mut state, RadiantEditorMessage::Undo);
        assert_eq!(params.editable_curve_snapshot(), origin);
        assert!(state.undo_history.is_empty());
        assert_eq!(state.redo_history.len(), 1);
    }

    #[test]
    fn undo_discards_active_curve_paint_before_restoring_history() {
        let params = Arc::new(PumpParams::new());
        let origin = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
            segments: vec![CurveSegment { tension: 0.0 }],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&origin);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint {
                sample: paint_sample(0.2, 0.2),
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint {
                sample: paint_sample(0.8, 0.8),
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
        );
        let painted = params.editable_curve_snapshot();
        assert_ne!(painted, origin);
        assert_eq!(state.undo_history.len(), 1);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint {
                sample: paint_sample(0.3, 0.9),
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint {
                sample: paint_sample(0.7, 0.1),
            }),
        );
        assert!(state.active_curve_paint.is_some());

        reduce_editor_message(&mut state, RadiantEditorMessage::Undo);
        assert!(state.active_curve_paint.is_none());
        assert_eq!(params.editable_curve_snapshot(), origin);
        assert!(state.undo_history.is_empty());
        assert_eq!(state.redo_history.len(), 1);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint {
                sample: Some(paint_sample(0.7, 0.1)),
            }),
        );
        assert_eq!(params.editable_curve_snapshot(), origin);
        assert!(state.undo_history.is_empty());
        assert_eq!(state.redo_history.len(), 1);
        assert_ne!(painted, origin);
    }

    #[test]
    fn redo_discards_active_curve_paint_before_restoring_history() {
        let params = Arc::new(PumpParams::new());
        let origin = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
            segments: vec![CurveSegment { tension: 0.0 }],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&origin);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint {
                sample: paint_sample(0.2, 0.2),
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint {
                sample: paint_sample(0.8, 0.8),
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
        );
        let painted = params.editable_curve_snapshot();
        reduce_editor_message(&mut state, RadiantEditorMessage::Undo);
        assert_eq!(params.editable_curve_snapshot(), origin);
        assert_eq!(state.redo_history.len(), 1);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint {
                sample: paint_sample(0.3, 0.9),
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint {
                sample: paint_sample(0.7, 0.1),
            }),
        );
        assert!(state.active_curve_paint.is_some());

        reduce_editor_message(&mut state, RadiantEditorMessage::Redo);
        assert!(state.active_curve_paint.is_none());
        assert_eq!(params.editable_curve_snapshot(), painted);
        assert_eq!(state.undo_history.len(), 1);
        assert!(state.redo_history.is_empty());

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint {
                sample: Some(paint_sample(0.7, 0.1)),
            }),
        );
        assert_eq!(params.editable_curve_snapshot(), painted);
        assert_eq!(state.undo_history.len(), 1);
        assert!(state.redo_history.is_empty());
    }

    #[test]
    fn curve_paint_reentry_splits_preview_and_commit_without_an_unobserved_chord() {
        let params = Arc::new(PumpParams::new());
        let origin = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.2, y: 0.2 },
                CurveNode { x: 0.5, y: 0.8 },
                CurveNode { x: 0.8, y: 0.3 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&origin);
        let mut state = editor_state(Arc::clone(&params));
        let first = paint_sample(0.2, 0.75);
        let second = paint_sample(0.3, 0.25);
        let outside = boundary_paint_sample(1.0, 0.55);
        let reentry = paint_sample(0.7, 0.65);
        let final_sample = paint_sample(0.8, 0.35);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: first }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint { sample: second }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaintOutside { sample: outside }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint { sample: reentry }),
        );
        let paint = state
            .active_curve_paint
            .as_ref()
            .expect("re-entry keeps painting active");
        let runs = paint.preview_runs();
        assert_eq!(runs.len(), 2);
        assert_eq!(
            runs[0].points().first().map(|point| point.position),
            Some(first.raw_position())
        );
        assert!(runs[0]
            .points()
            .iter()
            .any(|point| point.position == outside.raw_position()));
        assert_eq!(
            runs[1].points().first().map(|point| point.position),
            Some(outside.raw_position())
        );
        assert!(runs[1]
            .points()
            .iter()
            .any(|point| point.position == reentry.raw_position()));
        assert!(matches!(
            runs[1].points().first().map(|point| point.contact),
            Some(BoundaryContact::Edge(EdgeParameter {
                edge: BoundaryEdge::Right,
                ..
            }))
        ));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint {
                sample: final_sample,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaintOutside {
                sample: outside,
            }),
        );

        let painted = params.editable_curve_snapshot();
        assert_eq!(state.undo_history.len(), 1);
        assert!(painted.nodes.iter().any(|node| {
            (node.x - outside.node.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
                && (node.y - outside.node.y).abs() <= 0.08
        }));
        assert!(painted.nodes.iter().any(|node| {
            (node.x - reentry.node.x).abs() <= CURVE_PAINT_ASSERT_EPSILON
                && (node.y - reentry.node.y).abs() <= 0.08
        }));
        assert_curve_paint_topology_is_bounded(&painted);
    }

    #[test]
    fn curve_paint_full_capacity_boundary_extension_applies_boundary_candidate() {
        let params = Arc::new(PumpParams::new());
        let origin = EditableCurve {
            nodes: (0..MAX_EDITABLE_NODES)
                .map(|index| {
                    let x = index as f32 / (MAX_EDITABLE_NODES - 1) as f32;
                    CurveNode {
                        x,
                        y: 0.3 + 0.4 * x,
                    }
                })
                .collect(),
            segments: vec![CurveSegment { tension: 0.0 }; MAX_EDITABLE_NODES - 1],
            ..EditableCurve::default()
        }
        .normalized();
        assert_eq!(origin.nodes.len(), MAX_EDITABLE_NODES);
        params.set_editable_curve(&origin);
        let mut state = editor_state(Arc::clone(&params));
        let in_bounds = paint_sample(0.005, 0.2);
        let boundary = boundary_paint_sample(0.0, 0.8);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: in_bounds }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaintOutside { sample: boundary }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaintOutside {
                sample: boundary,
            }),
        );

        let painted = params.editable_curve_snapshot();
        assert_curve_paint_topology_is_bounded(&painted);
        assert_ne!(painted, origin);
        assert_eq!(
            painted.nodes.first().map(|node| node.y),
            Some(boundary.node.y)
        );
        assert_eq!(
            painted.nodes.last().map(|node| node.y),
            Some(boundary.node.y)
        );
        assert_eq!(state.undo_history.len(), 1);
        assert!(state.redo_history.is_empty());
        assert!(state.active_curve_paint.is_none());
    }

    #[test]
    fn curve_paint_effective_boundary_release_has_one_undo_and_cancel_has_none() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));
        let start = paint_sample(0.35, 0.2);
        let outside = boundary_paint_sample(0.0, 0.75);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: start }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaintOutside { sample: outside }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
        );
        assert_ne!(params.editable_curve_snapshot(), origin);
        assert_eq!(state.undo_history.len(), 1);
        assert!(state.redo_history.is_empty());

        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: start }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaintOutside { sample: outside }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::Cancel),
        );
        assert_eq!(params.editable_curve_snapshot(), origin);
        assert!(state.undo_history.is_empty());
        assert!(state.redo_history.is_empty());
        assert!(state.active_curve_paint.is_none());
    }

    #[test]
    fn radiant_editor_curve_paint_commits_one_localized_gesture() {
        let params = Arc::new(PumpParams::new());
        let origin = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.25, y: 0.7 },
                CurveNode { x: 0.5, y: 0.3 },
                CurveNode { x: 0.75, y: 0.7 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&origin);
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint {
                sample: paint_sample(0.3, 0.15),
            }),
        );
        assert!(state.active_curve_paint.is_some());
        assert_eq!(params.editable_curve_snapshot(), origin);
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint {
                sample: paint_sample(0.45, 0.85),
            }),
        );
        assert_eq!(params.editable_curve_snapshot(), origin);
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint {
                sample: Some(paint_sample(0.6, 0.2)),
            }),
        );

        let painted = params.editable_curve_snapshot();
        assert!(state.active_curve_paint.is_none());
        assert_eq!(state.undo_history.len(), 1);
        assert!(painted
            .nodes
            .iter()
            .any(|node| (node.x - 0.25).abs() < 1.0e-6));
        assert!(painted
            .nodes
            .iter()
            .any(|node| (node.x - 0.75).abs() < 1.0e-6));
        assert!(!painted
            .nodes
            .iter()
            .any(|node| (node.x - 0.5).abs() < 1.0e-6));
        assert!(painted
            .nodes
            .iter()
            .any(|node| (node.x - 0.3).abs() < 1.0e-6));
        assert!(painted
            .nodes
            .iter()
            .any(|node| (node.x - 0.6).abs() < 1.0e-6));
    }

    #[test]
    fn radiant_editor_curve_paint_commits_prior_samples_when_release_is_outside() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));
        let first = paint_sample(0.3, 0.15);
        let second = paint_sample(0.55, 0.8);
        let outside = boundary_paint_sample(1.0, 0.45);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: first }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint { sample: second }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaintOutside {
                sample: outside,
            }),
        );

        assert_ne!(params.editable_curve_snapshot(), origin);
        assert_eq!(state.undo_history.len(), 1);
        assert!(state.redo_history.is_empty());
        assert!(state.active_curve_paint.is_none());
    }

    #[test]
    fn radiant_editor_curve_paint_cancellation_and_noop_keep_history_unchanged() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));
        let sample = paint_sample(0.4, 0.2);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint { sample }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
        );
        assert_eq!(params.editable_curve_snapshot(), origin);
        assert!(state.undo_history.is_empty());
        assert!(state.redo_history.is_empty());

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint { sample }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint {
                sample: Some(sample),
            }),
        );
        assert_eq!(params.editable_curve_snapshot(), origin);
        assert!(state.undo_history.is_empty());
        assert!(state.redo_history.is_empty());
    }

    #[test]
    fn radiant_editor_curve_paint_noop_and_capacity_use_expected_history() {
        let params = Arc::new(PumpParams::new());
        let origin = EditableCurve {
            nodes: vec![CurveNode { x: 0.0, y: 0.5 }, CurveNode { x: 1.0, y: 0.5 }],
            segments: vec![CurveSegment { tension: 0.0 }],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&origin);
        let mut state = editor_state(Arc::clone(&params));
        let seed_start = paint_sample(0.2, 0.2);
        let seed_end = paint_sample(0.8, 0.8);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: seed_start }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint { sample: seed_end }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
        );
        reduce_editor_message(&mut state, RadiantEditorMessage::Undo);
        assert_eq!(params.editable_curve_snapshot(), origin);
        assert!(state.undo_history.is_empty());
        assert_eq!(state.redo_history.len(), 1);

        let no_op = paint_sample(0.4, 0.2);
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: no_op }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint {
                sample: Some(no_op),
            }),
        );
        assert_eq!(params.editable_curve_snapshot(), origin);
        assert!(state.undo_history.is_empty());
        assert_eq!(state.redo_history.len(), 1);

        let first = paint_sample(0.1, 0.1);
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressPaint { sample: first }),
        );
        for index in 1..70 {
            let x = 0.1 + index as f32 * 0.8 / 69.0;
            let y = if index % 2 == 0 { 0.1 } else { 0.9 };
            reduce_editor_message(
                &mut state,
                RadiantEditorMessage::Curve(CurvePreviewMessage::DragPaint {
                    sample: paint_sample(x, y),
                }),
            );
        }
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleasePaint { sample: None }),
        );
        assert_ne!(params.editable_curve_snapshot(), origin);
        assert_eq!(state.undo_history.len(), 1);
        assert!(state.redo_history.is_empty());
        assert!(state.active_curve_paint.is_none());
    }

    #[test]
    fn curve_preview_widget_shift_blank_press_starts_marquee_and_tracks_release() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let start = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.12, y: 0.94 });
        let end = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.64, y: 0.16 });
        let shift = PointerModifiers {
            shift: true,
            ..PointerModifiers::default()
        };
        let mut press_widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        assert_eq!(
            press_widget
                .handle_input(
                    bounds,
                    WidgetInput::PointerPress {
                        position: start,
                        button: PointerButton::Primary,
                        modifiers: shift,
                    },
                )
                .and_then(|output| output.typed_copied()),
            Some(CurvePreviewMessage::PressMarquee {
                start: CurvePreviewWidget::node_from_point(bounds, start),
            })
        );

        let mut drag_widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_active_curve_marquee(Some(ActiveCurveMarquee {
                start: CurvePreviewWidget::node_from_point(bounds, start),
                current: CurvePreviewWidget::node_from_point(bounds, start),
            }));
        assert_eq!(
            drag_widget
                .handle_input(bounds, WidgetInput::PointerMove { position: end })
                .and_then(|output| output.typed_copied()),
            Some(CurvePreviewMessage::DragMarquee {
                current: CurvePreviewWidget::node_from_point(bounds, end),
            })
        );
        assert_eq!(
            drag_widget
                .handle_input(
                    bounds,
                    WidgetInput::PointerRelease {
                        position: end,
                        button: PointerButton::Primary,
                        modifiers: shift,
                    },
                )
                .and_then(|output| output.typed_copied()),
            Some(CurvePreviewMessage::ReleaseMarquee {
                current: CurvePreviewWidget::node_from_point(bounds, end),
            })
        );
    }

    #[test]
    fn curve_preview_widget_cmd_shift_option_on_point_press_and_release_offsets_curve() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);
        let modifiers = PointerModifiers {
            shift: true,
            alt: true,
            command: true,
        };
        let mut press_widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);

        let press = press_widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers,
                },
            )
            .and_then(|output| output.typed_copied::<CurvePreviewMessage>());
        assert_eq!(
            press,
            Some(CurvePreviewMessage::PressCurveOffset {
                pointer_x: CurvePreviewWidget::offset_pointer_x(bounds, position),
                quantized: true,
            })
        );

        let mut release_widget =
            CurvePreviewWidget::new(curve, None, None, None, None, None, false)
                .with_active_curve_offset(Some(CurvePreviewWidget::offset_pointer_x(
                    bounds, position,
                )));
        let release = release_widget
            .handle_input(
                bounds,
                WidgetInput::PointerRelease {
                    position,
                    button: PointerButton::Primary,
                    modifiers,
                },
            )
            .and_then(|output| output.typed_copied::<CurvePreviewMessage>());
        assert_eq!(
            release,
            Some(CurvePreviewMessage::ReleaseCurveOffset {
                delta: 0.0,
                option_held: true,
            })
        );
    }

    #[test]
    fn curve_preview_widget_reserves_noninteractive_reference_gutter() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let curve_bounds = CurvePreviewWidget::curve_bounds(bounds);
        let gutter_position = Point::new(bounds.min.x + 10.0, bounds.min.y + 30.0);

        assert_eq!(curve_bounds.min.x, CURVE_REFERENCE_GUTTER_WIDTH);
        assert!(!curve_bounds.contains(gutter_position));
        assert_eq!(widget.hit_node(bounds, gutter_position), None);
        assert_eq!(widget.insert_node_at(bounds, gutter_position), None);
        assert!(widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: gutter_position,
                    button: PointerButton::Primary,
                    modifiers: Default::default(),
                },
            )
            .is_none());
        assert!(
            (CurvePreviewWidget::curve_point(bounds, curve.nodes[0]).x - curve_bounds.min.x).abs()
                < 1.0e-6
        );
        assert!(
            (CurvePreviewWidget::curve_point(bounds, *curve.nodes.last().unwrap()).x
                - (curve_bounds.max.x - 1.0))
                .abs()
                < 1.0e-6
        );
    }

    #[test]
    fn curve_preview_widget_push_through_threshold_tracks_actual_bounds() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, Some(1), None, None, None, None, false);
        let mut thresholds = Vec::new();

        for preview_width in [220.0, 396.0] {
            let bounds = Rect::from_xy_size(0.0, 0.0, preview_width, CURVE_PREVIEW_HEIGHT);
            let position = Point::new(bounds.max.x - 20.0, bounds.min.y + 30.0);
            let curve_pixel_width =
                (CurvePreviewWidget::curve_bounds(bounds).width().max(1.0) - 1.0).max(1.0);

            let drag = widget
                .handle_input(bounds, WidgetInput::PointerMove { position })
                .expect("active node move should emit a drag message");
            let drag_threshold = match drag.typed_copied() {
                Some(CurvePreviewMessage::DragNode {
                    push_through_threshold_x,
                    ..
                }) => push_through_threshold_x,
                other => panic!("unexpected drag output: {other:?}"),
            };
            assert!(
                (drag_threshold * curve_pixel_width - CURVE_NODE_PUSH_THROUGH_MARGIN_PX).abs()
                    < 1.0e-5
            );

            let release = widget
                .handle_input(
                    bounds,
                    WidgetInput::PointerRelease {
                        position,
                        button: PointerButton::Primary,
                        modifiers: Default::default(),
                    },
                )
                .expect("active node release should emit a release message");
            let release_threshold = match release.typed_copied() {
                Some(CurvePreviewMessage::ReleaseNode {
                    push_through_threshold_x,
                    ..
                }) => push_through_threshold_x,
                other => panic!("unexpected release output: {other:?}"),
            };
            assert!((release_threshold - drag_threshold).abs() < f32::EPSILON);
            thresholds.push(drag_threshold);
        }

        assert!(thresholds[0] > thresholds[1]);
    }

    #[test]
    fn curve_preview_widget_emits_preview_for_segment_hover() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let expected = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&curve, 0.2),
        };
        let position = CurvePreviewWidget::curve_point(bounds, expected);

        let output = widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .expect("segment hover should emit a preview message");

        match output.typed_copied() {
            Some(CurvePreviewMessage::Hover {
                node: None,
                preview_node: Some(node),
                segment: Some(1),
            }) => {
                assert!((node.x - expected.x).abs() < 1.0e-6);
                assert!((node.y - expected.y).abs() < 1.0e-6);
            }
            other => panic!("unexpected segment hover output: {other:?}"),
        }
    }

    #[test]
    fn curve_preview_widget_suppresses_preview_for_option_hovered_segment() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let expected = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&curve, 0.2),
        };
        let position = CurvePreviewWidget::curve_point(bounds, expected);

        let output = widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .expect("option line hover should emit segment hover state");

        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::Hover {
                node: None,
                preview_node: None,
                segment: Some(1),
            })
        );
    }

    #[test]
    fn curve_preview_widget_emits_insert_for_segment_press() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let expected = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&curve, 0.2),
        };
        let position = CurvePreviewWidget::curve_point(bounds, expected);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: Default::default(),
                },
            )
            .expect("segment press should emit an insert message");

        match output.typed_copied() {
            Some(CurvePreviewMessage::InsertNode { node, .. }) => {
                assert!((node.x - expected.x).abs() < 1.0e-6);
                assert!((node.y - expected.y).abs() < 1.0e-6);
            }
            other => panic!("unexpected segment press output: {other:?}"),
        }
    }

    #[test]
    fn curve_preview_widget_emits_segment_press_when_option_held() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let expected = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&curve, 0.2),
        };
        let position = CurvePreviewWidget::curve_point(bounds, expected);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        alt: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .expect("option segment press should begin segment drag");

        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::PressSegment { index: 1, position })
        );
    }

    #[test]
    fn curve_preview_widget_option_press_on_empty_canvas_is_no_op() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.72, y: 0.18 });

        assert!(widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        alt: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .is_none());
    }

    #[test]
    fn curve_preview_widget_cmd_shift_press_on_empty_canvas_starts_offset() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.72, y: 0.18 });

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        command: true,
                        shift: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .expect("Cmd+Shift empty-canvas press should start an offset");
        assert!(matches!(
            output.typed_copied(),
            Some(CurvePreviewMessage::PressCurveOffset {
                quantized: false,
                ..
            })
        ));
    }

    #[test]
    fn curve_preview_widget_cmd_shift_press_on_node_starts_exclusive_offset() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        command: true,
                        shift: true,
                        alt: true,
                    },
                },
            )
            .expect("Cmd+Shift node press should start whole-curve offset");
        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::PressCurveOffset {
                pointer_x: CurvePreviewWidget::offset_pointer_x(bounds, position),
                quantized: true,
            })
        );
    }

    #[test]
    fn curve_preview_widget_cmd_only_press_does_not_reuse_stale_shift_hover() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_command_hover_held(true)
            .with_shift_hover_held(true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.72, y: 0.18 });

        let output = widget.handle_input(
            bounds,
            WidgetInput::PointerPress {
                position,
                button: PointerButton::Primary,
                modifiers: PointerModifiers {
                    command: true,
                    ..PointerModifiers::default()
                },
            },
        );
        assert!(!matches!(
            output.and_then(|output| output.typed_copied()),
            Some(CurvePreviewMessage::PressCurveOffset { .. })
        ));
    }

    #[test]
    fn curve_preview_widget_shift_only_press_does_not_reuse_stale_command_hover() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_command_hover_held(true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.72, y: 0.18 });

        let output = widget.handle_input(
            bounds,
            WidgetInput::PointerPress {
                position,
                button: PointerButton::Primary,
                modifiers: PointerModifiers {
                    shift: true,
                    ..PointerModifiers::default()
                },
            },
        );
        assert!(!matches!(
            output.and_then(|output| output.typed_copied()),
            Some(CurvePreviewMessage::PressCurveOffset { .. })
        ));
    }

    #[test]
    fn radiant_editor_cmd_shift_offset_moves_only_phase_and_commits_on_release() {
        let params = Arc::new(PumpParams::new());
        let origin = params.editable_curve_snapshot();
        let origin_table = params.curve_snapshot();
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
                pointer_x: 0.4,
                quantized: false,
            }),
        );
        assert_eq!(state.undo_history.len(), 1);
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragCurveOffset { delta: 0.25 }),
        );
        assert!(state.active_curve_offset.is_some());
        assert_eq!(params.editable_curve_snapshot(), origin);
        assert_eq!(params.curve_snapshot(), origin_table);
        assert!((params.phase_offset() - 0.25).abs() < 1.0e-6);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseCurveOffset {
                delta: 0.25,
                option_held: false,
            }),
        );
        assert!(state.active_curve_offset.is_none());
        assert_eq!(params.editable_curve_snapshot(), origin);
        assert!((params.phase_offset() - 0.25).abs() < 1.0e-6);
        assert_eq!(state.undo_history.len(), 1);
    }

    #[test]
    fn radiant_editor_cmd_shift_offset_cancel_restores_auditioned_origin() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
                pointer_x: 0.4,
                quantized: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragCurveOffset { delta: 0.25 }),
        );
        assert!((params.phase_offset() - 0.25).abs() < 1.0e-6);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::Cancel),
        );

        assert!(params.phase_offset().abs() < f32::EPSILON);
        assert!(state.active_curve_offset.is_none());
        assert!(state.preview_curve_offset.is_none());
    }

    #[test]
    fn radiant_editor_cmd_shift_offset_snaps_immediately_when_option_is_pressed() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));
        let raw_delta = 0.17;
        let width = (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
                pointer_x: 0.4,
                quantized: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragCurveOffset { delta: raw_delta }),
        );
        assert!((params.phase_offset() - raw_delta).abs() < 1.0e-6);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                option_held: true,
                command_held: true,
                shift_held: true,
            }),
        );
        let snapped = resolve_curve_offset(
            state.params.sync_division(),
            width,
            state.params.swing(),
            0.0,
            raw_delta,
            true,
        );
        assert!(state
            .active_curve_offset
            .as_ref()
            .is_some_and(|drag| drag.quantized));
        assert!((params.phase_offset() - snapped).abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_cmd_shift_offset_reverses_to_free_mode_immediately() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));
        let raw_delta = 0.17;

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
                pointer_x: 0.4,
                quantized: true,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragCurveOffset { delta: raw_delta }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: true,
                shift_held: true,
            }),
        );
        assert!(state
            .active_curve_offset
            .as_ref()
            .is_some_and(|drag| !drag.quantized));
        assert!((params.phase_offset() - raw_delta).abs() < 1.0e-6);
    }

    #[test]
    fn radiant_editor_cmd_shift_offset_uses_option_state_at_release_without_new_move() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));
        let raw_delta = 0.17;
        let width = (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0);
        let snapped = resolve_curve_offset(
            state.params.sync_division(),
            width,
            state.params.swing(),
            0.0,
            raw_delta,
            true,
        );

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
                pointer_x: 0.4,
                quantized: false,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::ReleaseCurveOffset {
                delta: raw_delta,
                option_held: true,
            }),
        );

        assert!((params.phase_offset() - snapped).abs() < 1.0e-6);
        assert!(state.active_curve_offset.is_none());
        assert!(state.preview_curve_offset.is_none());
    }

    #[test]
    fn radiant_editor_option_offset_snap_uses_the_absolute_grid_position() {
        let params = Arc::new(PumpParams::new());
        params.set_phase_offset(0.18);
        let mut state = editor_state(Arc::clone(&params));
        let width = (WINDOW_WIDTH as f32 - SURFACE_PADDING * 2.0).max(1.0);
        let delta = 0.12;
        let expected = resolve_curve_offset(
            state.params.sync_division(),
            width,
            state.params.swing(),
            0.18,
            delta,
            true,
        );

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::PressCurveOffset {
                pointer_x: 0.4,
                quantized: true,
            }),
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragCurveOffset { delta }),
        );

        assert!((params.phase_offset() - expected).abs() < 1.0e-6);
    }

    #[test]
    fn offset_bar_double_click_resets_the_automatable_phase_offset() {
        let params = Arc::new(PumpParams::new());
        params.set_phase_offset(0.43);
        let mut state = editor_state(Arc::clone(&params));
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let bar = CurvePreviewWidget::offset_bar_bounds(bounds);
        let mut widget = CurvePreviewWidget::new(
            params.editable_curve_snapshot(),
            None,
            None,
            None,
            None,
            None,
            false,
        );

        let message = widget
            .handle_input(
                bounds,
                WidgetInput::primary_double_click(Point::new(
                    (bar.min.x + bar.max.x) * 0.5,
                    (bar.min.y + bar.max.y) * 0.5,
                )),
            )
            .and_then(|output| output.typed_copied());
        assert_eq!(message, Some(CurvePreviewMessage::ResetCurveOffset));
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(message.expect("offset reset message")),
        );
        assert!(params.phase_offset().abs() < f32::EPSILON);
    }

    #[test]
    fn curve_preview_widget_emits_insert_for_empty_canvas_press() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let expected = CurveNode { x: 0.72, y: 0.18 };
        let position = CurvePreviewWidget::curve_point(bounds, expected);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: Default::default(),
                },
            )
            .expect("empty canvas press should emit an insert message");

        match output.typed_copied() {
            Some(CurvePreviewMessage::InsertNode { node, .. }) => {
                assert!((node.x - expected.x).abs() < 1.0e-6);
                assert!((node.y - expected.y).abs() < 1.0e-6);
            }
            other => panic!("unexpected empty canvas press output: {other:?}"),
        }
    }

    #[test]
    fn curve_preview_widget_unmodified_blank_press_ignores_stale_shift_hover() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_shift_hover_held(true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let expected = CurveNode { x: 0.72, y: 0.18 };
        let position = CurvePreviewWidget::curve_point(bounds, expected);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers::default(),
                },
            )
            .expect("blank canvas press should emit an insert message");

        assert!(matches!(
            output.typed_copied(),
            Some(CurvePreviewMessage::InsertNode { .. })
        ));
    }

    #[test]
    fn host_sound_switch_clears_curve_selection_before_projecting_new_curve() {
        let params = Arc::new(PumpParams::new());
        let curve_a = params.editable_curve_snapshot();
        params.copy_active_to_inactive();
        params.set_active_sound(SoundSide::B);
        let curve_b = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.1 },
                CurveNode { x: 0.2, y: 0.8 },
                CurveNode { x: 0.45, y: 0.3 },
                CurveNode { x: 0.7, y: 0.9 },
                CurveNode { x: 1.0, y: 0.2 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 4],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve_b);
        params.set_active_sound(SoundSide::A);
        assert_eq!(params.editable_curve_snapshot(), curve_a);

        let mut state = editor_state(Arc::clone(&params));
        state.selected_curve_nodes = vec![1];
        state.active_curve_marquee = Some(ActiveCurveMarquee {
            start: CurveNode { x: 0.1, y: 0.9 },
            current: CurveNode { x: 0.8, y: 0.1 },
        });

        crate::params::apply_param_event(params.as_ref(), PARAM_SOUND_ID, 1.0);
        let _ = project_editor_surface(&mut state);

        assert!(state.selected_curve_nodes.is_empty());
        assert!(state.active_curve_marquee.is_none());
        assert_eq!(params.editable_curve_snapshot(), curve_b);
    }

    #[test]
    fn radiant_editor_canvas_insert_can_drag_inserted_node() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(Arc::clone(&params));

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::InsertNode {
                node: CurveNode { x: 0.72, y: 0.18 },
                command_held: false,
            }),
        );
        let inserted = state
            .active_curve_node
            .expect("inserted node should become active");

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Curve(CurvePreviewMessage::DragNode {
                index: inserted,
                node: CurveNode { x: 0.62, y: 0.42 },
                push_through_threshold_x: test_curve_push_through_threshold_x(),
            }),
        );
        let curve = params.editable_curve_snapshot();
        assert!((curve.nodes[inserted].x - 0.62).abs() < 1.0e-6);
        assert!((curve.nodes[inserted].y - 0.42).abs() < 1.0e-6);
    }

    #[test]
    fn curve_preview_widget_command_insert_snaps_time_and_preserves_gain() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_sync_division(6)
            .with_swing(1.0);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let raw = CurveNode { x: 0.34, y: 0.08 };
        let position = CurvePreviewWidget::curve_point(bounds, raw);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        command: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .expect("command empty-canvas press should insert a snapped point");

        let Some(CurvePreviewMessage::InsertNode {
            node,
            command_held: true,
        }) = output.typed_copied()
        else {
            panic!(
                "unexpected command insertion output: {:?}",
                output.typed_copied::<CurvePreviewMessage>()
            );
        };
        let width = CurvePreviewWidget::curve_bounds(bounds).width();
        assert!(
            (node.x - snap_curve_time_to_beat_grid_with_swing(6, width, raw.x, 1.0)).abs() < 1.0e-6
        );
        assert!((node.y - raw.y).abs() < 1.0e-2);
    }

    #[test]
    fn radiant_editor_command_press_and_release_update_point_snap_mid_drag() {
        let params = Arc::new(PumpParams::new());
        params.set_sync_division(6.0);
        params.set_swing(1.0);
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.8 },
                CurveNode { x: 0.25, y: 0.4 },
                CurveNode { x: 0.75, y: 0.6 },
                CurveNode { x: 1.0, y: 0.8 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 3],

            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 1,
                pointer: curve.nodes[1],
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: true,
                shift_held: false,
            },
        );

        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let raw = CurveNode { x: 0.34, y: 0.7 };
        let position = CurvePreviewWidget::curve_point(bounds, raw);
        let mut snapped_widget = CurvePreviewWidget::new(
            params.editable_curve_snapshot(),
            state.active_curve_node,
            None,
            None,
            None,
            None,
            false,
        )
        .with_command_hover_held(true)
        .with_sync_division(6)
        .with_swing(1.0);
        let snapped_message = snapped_widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .and_then(|output| output.typed_copied::<CurvePreviewMessage>())
            .expect("active command drag should emit a snapped node move");
        reduce_curve_message(&mut state, snapped_message);

        let width = CurvePreviewWidget::curve_bounds(bounds).width();
        let snapped = params.editable_curve_snapshot().nodes[1];
        assert!(
            (snapped.x - snap_curve_time_to_beat_grid_with_swing(6, width, raw.x, 1.0)).abs()
                < 1.0e-6
        );

        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ModifiersChanged {
                option_held: false,
                command_held: false,
                shift_held: false,
            },
        );
        let mut plain_widget = CurvePreviewWidget::new(
            params.editable_curve_snapshot(),
            state.active_curve_node,
            None,
            None,
            None,
            None,
            false,
        )
        .with_sync_division(6);
        let plain_message = plain_widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .and_then(|output| output.typed_copied::<CurvePreviewMessage>())
            .expect("active plain drag should emit a continuous node move");
        reduce_curve_message(&mut state, plain_message);

        let continuous = params.editable_curve_snapshot().nodes[1];
        assert!((continuous.x - raw.x).abs() < 1.0e-2);
        assert!((continuous.y - snapped.y).abs() < 1.0e-2);
    }

    #[test]
    fn radiant_editor_command_release_uses_swung_snap() {
        let params = Arc::new(PumpParams::new());
        params.set_sync_division(6.0);
        params.set_swing(1.0);
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.8 },
                CurveNode { x: 0.25, y: 0.4 },
                CurveNode { x: 0.75, y: 0.6 },
                CurveNode { x: 1.0, y: 0.8 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 3],
            ..EditableCurve::default()
        }
        .normalized();
        params.set_editable_curve(&curve);
        let mut state = editor_state(Arc::clone(&params));
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::PressNode {
                index: 1,
                pointer: curve.nodes[1],
                shift_held: false,
                option_held: false,
                command_held: false,
            },
        );

        let raw = CurveNode { x: 0.34, y: 0.7 };
        reduce_curve_message(
            &mut state,
            CurvePreviewMessage::ReleaseNode {
                index: 1,
                node: raw,
                push_through_threshold_x: test_curve_push_through_threshold_x(),
                shift_held: false,
                option_held: false,
                command_held: true,
            },
        );

        let width =
            curve_width_from_push_through_threshold_x(test_curve_push_through_threshold_x());
        let released = params.editable_curve_snapshot().nodes[1];
        assert!(
            (released.x - snap_curve_time_to_beat_grid_with_swing(6, width, raw.x, 1.0)).abs()
                < 1.0e-6,
            "released {}, expected {} width {}",
            released.x,
            snap_curve_time_to_beat_grid_with_swing(6, width, raw.x, 1.0),
            width
        );
    }

    #[test]
    fn curve_preview_widget_tracks_option_hovered_segment() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(
            bounds,
            CurveNode {
                x: 0.2,
                y: sample_editable_curve(&curve, 0.2),
            },
        );

        let output = widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .expect("line hover should emit hover state");
        assert!(matches!(
            output.typed_copied(),
            Some(CurvePreviewMessage::Hover {
                segment: Some(1),
                ..
            })
        ));

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerModifiersChanged {
                    modifiers: PointerModifiers {
                        alt: true,
                        ..PointerModifiers::default()
                    },
                },
            )
            .expect("option press should emit option-hover state");
        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::ModifiersChanged {
                option_held: true,
                command_held: false,
                shift_held: false,
            })
        );
    }

    #[test]
    fn curve_preview_widget_command_press_prefers_point_then_segment_then_empty_canvas() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let modifiers = PointerModifiers {
            command: true,
            ..PointerModifiers::default()
        };

        let mut point_widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let point = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);
        let point_output = point_widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: point,
                    button: PointerButton::Primary,
                    modifiers,
                },
            )
            .expect("command point press should remain a point gesture");
        assert_eq!(
            point_output.typed_copied(),
            Some(CurvePreviewMessage::PressNode {
                index: 1,
                pointer: CurvePreviewWidget::node_from_point(bounds, point),
                shift_held: false,
                option_held: false,
                command_held: true,
            })
        );

        let mut segment_widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let segment = CurvePreviewWidget::curve_point(
            bounds,
            CurveNode {
                x: 0.2,
                y: sample_editable_curve(&curve, 0.2),
            },
        );
        let segment_output = segment_widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: segment,
                    button: PointerButton::Primary,
                    modifiers,
                },
            )
            .expect("command line press should start grouped movement");
        assert!(matches!(
            segment_output.typed_copied(),
            Some(CurvePreviewMessage::PressSegmentMove { index: 1, .. })
        ));

        let mut empty_widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false);
        let empty = CurvePreviewWidget::curve_point(bounds, CurveNode { x: 0.72, y: 0.18 });
        let empty_output = empty_widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position: empty,
                    button: PointerButton::Primary,
                    modifiers,
                },
            )
            .expect("command empty-canvas press should retain insertion behavior");
        assert!(matches!(
            empty_output.typed_copied(),
            Some(CurvePreviewMessage::InsertNode { .. })
        ));
    }

    #[test]
    fn curve_preview_widget_shift_and_command_compose_on_point_press() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false);

        let output = widget
            .handle_input(
                bounds,
                WidgetInput::PointerPress {
                    position,
                    button: PointerButton::Primary,
                    modifiers: PointerModifiers {
                        command: true,
                        shift: true,
                        alt: false,
                    },
                },
            )
            .expect("Cmd+Shift point press should start whole-curve offset");

        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::PressCurveOffset {
                pointer_x: CurvePreviewWidget::offset_pointer_x(bounds, position),
                quantized: false,
            })
        );
    }

    #[test]
    fn curve_preview_widget_emits_hover_for_hit_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);

        let output = widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .expect("node hover should emit hover state");

        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::Hover {
                node: Some(1),
                preview_node: None,
                segment: None,
            })
        );
    }

    #[test]
    fn curve_preview_widget_clears_node_hover_off_hit_target() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, Some(1), None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let node_position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);
        let position = Point::new(
            node_position.x + CURVE_NODE_HIT_RADIUS + 2.0,
            node_position.y,
        );

        let output = widget
            .handle_input(bounds, WidgetInput::PointerMove { position })
            .expect("moving off the node should clear hover state");

        assert!(matches!(
            output.typed_copied(),
            Some(CurvePreviewMessage::Hover { node: None, .. })
        ));
    }

    #[test]
    fn curve_preview_widget_emits_delete_for_double_clicked_interior_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[1]);

        let output = widget
            .handle_input(bounds, WidgetInput::primary_double_click(position))
            .expect("interior node double-click should emit delete");

        assert_eq!(
            output.typed_copied(),
            Some(CurvePreviewMessage::DeleteNode { index: 1 })
        );
    }

    #[test]
    fn curve_preview_widget_ignores_double_clicked_endpoint() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut widget =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let position = CurvePreviewWidget::curve_point(bounds, curve.nodes[0]);

        assert_eq!(
            widget.handle_input(bounds, WidgetInput::primary_double_click(position)),
            None
        );
    }

    #[test]
    fn curve_preview_widget_emits_delete_for_focused_selection() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.5 },
                CurveNode { x: 0.4, y: 0.3 },
                CurveNode { x: 1.0, y: 0.5 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 2],
            ..EditableCurve::default()
        }
        .normalized();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let mut widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_selected_curve_nodes(&[1]);
        assert!(widget
            .handle_input(bounds, WidgetInput::FocusChanged(true))
            .is_none());
        assert!(widget.common().state.focused);

        for key in [WidgetKey::Delete, WidgetKey::Backspace] {
            let output = widget
                .handle_input(bounds, WidgetInput::KeyPress(key))
                .expect("focused selected curve should handle deletion");
            assert_eq!(
                output.typed_copied(),
                Some(CurvePreviewMessage::DeleteSelectedNodes)
            );
        }
    }

    #[test]
    fn curve_preview_widget_paints_preview_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let preview = CurveNode {
            x: 0.2,
            y: sample_editable_curve(&curve, 0.2),
        };
        let widget = CurvePreviewWidget::new(curve, None, None, None, Some(preview), None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::FillRect(fill)
                    if fill.color == theme.accent_warning
                        && (fill.rect.width() - CURVE_PREVIEW_NODE_SIZE).abs() < 1.0e-5
                        && (fill.rect.height() - CURVE_PREVIEW_NODE_SIZE).abs() < 1.0e-5
            )
        }));
    }

    #[test]
    fn curve_preview_widget_paints_the_active_ordered_run() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let samples = vec![
            paint_sample(0.18, 0.22),
            paint_sample(0.33, 0.76),
            paint_sample(0.51, 0.42),
        ];
        let mut recorder = StrokeRecorder::new(RectBounds {
            min: RectPoint { x: 0.0, y: 0.0 },
            max: RectPoint { x: 1.0, y: 1.0 },
        });
        for sample in &samples {
            recorder.observe(sample.raw_position());
        }
        let runs = recorder.runs().to_vec();
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_active_curve_paint(true)
            .with_curve_paint_runs(Some(runs.clone()));
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let preview = primitives.iter().find_map(|primitive| match primitive {
            PaintPrimitive::StrokePolyline(polyline)
                if polyline.color == theme.accent_copper
                    && (polyline.width - CURVE_PAINT_PREVIEW_WIDTH).abs() < 1.0e-6 =>
            {
                Some(polyline)
            }
            _ => None,
        });
        let preview = preview.expect("active paint should render a freehand stroke");
        assert_ne!(preview.color, theme.accent_mint);
        assert_eq!(preview.points.len(), runs[0].points().len());
        for (point, captured) in preview.points.iter().zip(runs[0].points()) {
            assert_eq!(
                *point,
                CurvePreviewWidget::curve_point(
                    bounds,
                    CurveNode {
                        x: captured.position.x,
                        y: captured.position.y,
                    },
                )
            );
        }
    }

    #[test]
    fn curve_preview_widget_paints_ordered_runs_as_separate_polylines() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let runs = vec![
            recorded_run([RectPoint { x: 0.2, y: 0.2 }, RectPoint { x: 0.35, y: 0.8 }]),
            recorded_run([RectPoint { x: 0.35, y: 0.8 }, RectPoint { x: 0.0, y: 0.6 }]),
            recorded_run([RectPoint { x: 0.7, y: 0.4 }]),
        ];
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_active_curve_paint(true)
            .with_curve_paint_runs(Some(runs));
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let previews = primitives
            .iter()
            .filter_map(|primitive| match primitive {
                PaintPrimitive::StrokePolyline(polyline)
                    if polyline.color == theme.accent_copper
                        && (polyline.width - CURVE_PAINT_PREVIEW_WIDTH).abs() < 1.0e-6 =>
                {
                    Some(polyline)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(previews.len(), 2);
        assert_eq!(previews[0].points.len(), 2);
        assert_eq!(previews[1].points.len(), 2);
    }

    #[test]
    fn curve_preview_widget_paints_option_hovered_segment() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, Some(1), true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokePolyline(polyline)
                    if polyline.color == theme.accent_warning
                        && (polyline.width - 2.975).abs() < 1.0e-6
                        && polyline.points.len() > 2
            )
        }));
    }

    #[test]
    fn curve_preview_widget_paints_command_hovered_segment_in_dedicated_color() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, Some(1), false)
            .with_command_hover_held(true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        assert_ne!(CURVE_SEGMENT_MOVE_COLOR, theme.accent_warning);
        assert_ne!(CURVE_SEGMENT_MOVE_COLOR, theme.accent_mint);
        assert_ne!(CURVE_SEGMENT_MOVE_COLOR, CURVE_PLAYHEAD_CORE_COLOR);
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokePolyline(polyline)
                    if polyline.color == CURVE_SEGMENT_MOVE_COLOR
                        && (polyline.width - 2.975).abs() < 1.0e-6
                        && polyline.points.len() > 2
            )
        }));
    }

    #[test]
    fn curve_preview_widget_splits_wrapping_segment_highlight_at_display_seam() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.5 },
                CurveNode { x: 0.4, y: 0.2 },
                CurveNode { x: 1.0, y: 0.8 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 2],
            ..EditableCurve::default()
        }
        .normalized();
        let widget = CurvePreviewWidget::new(curve, None, Some(1), None, None, None, false)
            .with_phase_offset(0.5);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let highlights: Vec<_> = primitives
            .iter()
            .filter_map(|primitive| match primitive {
                PaintPrimitive::StrokePolyline(polyline)
                    if polyline.color == theme.accent_warning
                        && (polyline.width - 2.975).abs() < 1.0e-6 =>
                {
                    Some(polyline)
                }
                _ => None,
            })
            .collect();
        assert_eq!(highlights.len(), 2, "wrapping segment should split at seam");
        let curve_width = CurvePreviewWidget::curve_bounds(bounds).width();
        for highlight in highlights {
            assert!(highlight.points.len() > 1);
            for pair in highlight.points.windows(2) {
                assert!(
                    pair[1].x >= pair[0].x - 1.0e-6,
                    "highlight polyline must not run right-to-left: {pair:?}"
                );
                assert!(
                    (pair[1].x - pair[0].x).abs() < curve_width * 0.5,
                    "highlight polyline contains a seam bridge: {pair:?}"
                );
            }
        }
    }

    #[test]
    fn curve_preview_widget_highlights_entire_curve_during_offset_drag() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_active_curve_offset(Some(0.4));
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokePolyline(polyline)
                    if polyline.color == CURVE_OFFSET_MOVE_COLOR
                        && (polyline.width - 2.55).abs() < 1.0e-6
                        && polyline.points.len() > 2
            )
        }));
    }

    #[test]
    fn curve_preview_widget_uses_a_lighter_yellow_hover_for_offset_mode() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_command_hover_held(true)
            .with_shift_hover_held(true);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let normal_index = primitives.iter().position(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokePolyline(polyline)
                    if polyline.color == theme.accent_mint
                        && (polyline.width - 1.7).abs() < 1.0e-6
                        && polyline.points.len() > 2
            )
        });
        let highlight_index = primitives.iter().position(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokePolyline(polyline)
                    if polyline.color == CURVE_OFFSET_HOVER_COLOR
                        && (polyline.width - 2.975).abs() < 1.0e-6
                        && polyline.points.len() > 2
            )
        });
        let normal_index = normal_index.expect("normal curve stroke should be rendered");
        let highlight_index = highlight_index.expect("offset hover highlight should be rendered");
        assert!(normal_index < highlight_index);
        let (normal, highlight) = match (&primitives[normal_index], &primitives[highlight_index]) {
            (PaintPrimitive::StrokePolyline(normal), PaintPrimitive::StrokePolyline(highlight)) => {
                (normal, highlight)
            }
            _ => unreachable!("indices identify curve strokes"),
        };
        assert_eq!(normal.points, highlight.points);
        assert_ne!(normal.color, highlight.color);
        assert!(highlight.width > normal.width);
    }

    #[test]
    fn curve_preview_widget_paints_smoothed_secondary_curve_only_when_enabled() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut raw_primitives = Vec::new();
        CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false)
            .with_smooth(0.0)
            .append_paint(
                &mut raw_primitives,
                bounds,
                &LayoutOutput::default(),
                &theme,
            );
        assert!(!raw_primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokePolyline(polyline)
                    if polyline.color == theme.text_primary.with_alpha(188)
                        && (polyline.width - 1.275).abs() < 1.0e-6
            )
        }));

        let mut smoothed_primitives = Vec::new();
        CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_smooth(0.75)
            .append_paint(
                &mut smoothed_primitives,
                bounds,
                &LayoutOutput::default(),
                &theme,
            );
        assert!(smoothed_primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokePolyline(polyline)
                    if polyline.color == theme.text_primary.with_alpha(188)
                        && (polyline.width - 1.275).abs() < 1.0e-6
                        && polyline.points.len() == CURVE_SAMPLE_COUNT + 1
            )
        }));
    }

    #[test]
    fn smooth_preview_radius_preserves_legacy_range_and_reaches_stronger_tail() {
        assert_eq!(CurvePreviewWidget::smooth_preview_radius(0.0), 0);
        assert_eq!(CurvePreviewWidget::smooth_preview_radius(0.5), 4);
        assert_eq!(CurvePreviewWidget::smooth_preview_radius(0.75), 6);
        assert_eq!(CurvePreviewWidget::smooth_preview_radius(1.0), 20);

        let mut previous = CurvePreviewWidget::smooth_preview_radius(0.75);
        for step in 1..=100 {
            let amount = 0.75 + 0.25 * step as f32 / 100.0;
            let current = CurvePreviewWidget::smooth_preview_radius(amount);
            assert!(
                current >= previous,
                "preview radius must be monotonic at {amount}"
            );
            previous = current;
        }
    }

    #[test]
    fn curve_preview_widget_offsets_the_curve_beneath_a_fixed_playhead() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let phase_offset = 0.25;
        let widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false)
            .with_phase_offset(phase_offset)
            .with_playhead_phase(Some(0.5));

        let points = widget.sample_curve_points(bounds);
        let expected = CurvePreviewWidget::curve_point(
            bounds,
            CurveNode {
                x: 0.5,
                y: sample_editable_curve(&curve, 0.5 - phase_offset),
            },
        );
        let midpoint = points
            .iter()
            .copied()
            .min_by(|left, right| {
                (left.x - expected.x)
                    .abs()
                    .total_cmp(&(right.x - expected.x).abs())
            })
            .expect("curve sampling should produce points");
        assert!((midpoint.x - expected.x).abs() <= 2.0);
        assert!((midpoint.y - expected.y).abs() <= 4.0);

        let mut primitives = Vec::new();
        widget.append_paint(
            &mut primitives,
            bounds,
            &LayoutOutput::default(),
            &ThemeTokens::default(),
        );
        let playhead = primitives.iter().find_map(|primitive| match primitive {
            PaintPrimitive::StrokePolyline(line)
                if line.color == CURVE_PLAYHEAD_CORE_COLOR && line.points.len() == 2 =>
            {
                Some(line.points[0])
            }
            _ => None,
        });
        assert_eq!(playhead.map(|point| point.x), Some(expected.x));
    }

    #[test]
    fn curve_preview_widget_samples_authored_nodes_at_exact_display_positions() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.137, y: 0.16 },
                CurveNode { x: 0.863, y: 0.74 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 1.0 }; 3],
            ..EditableCurve::default()
        }
        .normalized();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_phase_offset(0.271);
        let points = widget.sample_curve_points(bounds);

        for node in widget.curve.nodes.iter().copied() {
            let expected = widget.display_curve_point(bounds, node);
            assert!(
                points.iter().any(|point| {
                    (point.x - expected.x).abs() < 1.0e-6 && (point.y - expected.y).abs() < 1.0e-6
                }),
                "authored node {node:?} should be represented exactly"
            );
        }
    }

    #[test]
    fn curve_preview_widget_keeps_dragged_nodes_on_the_selected_seam_edge() {
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let curve_bounds = CurvePreviewWidget::curve_bounds(bounds);
        let right_bottom = Point::new(curve_bounds.max.x, curve_bounds.max.y);
        let left_bottom = Point::new(curve_bounds.min.x, curve_bounds.max.y);
        let midpoint = (curve_bounds.min.x + curve_bounds.max.x) * 0.5;

        for phase_offset in [0.0, 0.25] {
            let curve = PumpParams::new().editable_curve_snapshot();
            let widget = CurvePreviewWidget::new(curve, Some(1), None, None, None, None, false)
                .with_phase_offset(phase_offset);
            let right_node = widget.raw_node_from_display_point(bounds, right_bottom);
            let left_node = widget.raw_node_from_display_point(bounds, left_bottom);
            let right_point = widget.display_curve_point(bounds, right_node);
            let left_point = widget.display_curve_point(bounds, left_node);

            assert!(
                right_point.x > midpoint,
                "a node dragged to the bottom-right must remain on the right side at phase offset {phase_offset}: raw={right_node:?}, display={right_point:?}"
            );
            assert!(
                left_point.x < midpoint,
                "a node dragged to the bottom-left must remain on the left side at phase offset {phase_offset}: raw={left_node:?}, display={left_point:?}"
            );
            assert!((right_point.y - left_point.y).abs() < 1.0e-6);
        }
    }

    #[test]
    fn curve_preview_widget_crosses_the_active_offset_seam_without_locking() {
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let phase_offset = 0.25;
        let seam_position = CurvePreviewWidget::curve_point(
            bounds,
            CurveNode {
                x: phase_offset,
                y: 0.0,
            },
        );

        let right_side_curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.1, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 2],
            ..EditableCurve::default()
        }
        .normalized();
        let right_side_widget =
            CurvePreviewWidget::new(right_side_curve, Some(1), None, None, None, None, false)
                .with_phase_offset(phase_offset);
        let right_side_target =
            right_side_widget.raw_node_from_display_point(bounds, seam_position);
        assert!(
            right_side_target.x > 0.5,
            "crossing the offset seam from the right must continue toward raw phase 1: {right_side_target:?}"
        );

        let left_side_curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 1.0 },
                CurveNode { x: 0.9, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 2],
            ..EditableCurve::default()
        }
        .normalized();
        let left_side_widget =
            CurvePreviewWidget::new(left_side_curve, Some(1), None, None, None, None, false)
                .with_phase_offset(phase_offset);
        let left_side_target = left_side_widget.raw_node_from_display_point(bounds, seam_position);
        assert!(
            left_side_target.x < 0.5,
            "crossing the offset seam from the left must continue toward raw phase 0: {left_side_target:?}"
        );
    }

    #[test]
    fn curve_preview_widget_smoothed_curve_uses_display_phase_offset() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let phase_offset = 0.237;
        let widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false)
            .with_phase_offset(phase_offset)
            .with_smooth(0.75);
        let points = widget.sample_smoothed_curve_points(bounds);
        let midpoint = points[CURVE_SAMPLE_COUNT / 2];
        let expected = CurvePreviewWidget::curve_point(
            bounds,
            CurveNode {
                x: 0.5,
                y: widget.sample_smoothed_curve(0.5 - phase_offset),
            },
        );
        assert!((midpoint.x - expected.x).abs() < 1.0e-6);
        assert!((midpoint.y - expected.y).abs() < 1.0e-6);
        assert_ne!(
            midpoint.y,
            CurvePreviewWidget::curve_point(
                bounds,
                CurveNode {
                    x: 0.5,
                    y: widget.sample_smoothed_curve(0.5),
                },
            )
            .y
        );
    }

    #[test]
    fn curve_preview_widget_segment_highlight_uses_display_phase_offset() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false)
            .with_phase_offset(0.237);
        let points = widget.sample_segment_points(bounds, 1);
        let left = curve.nodes[1];
        let right = curve.nodes[2];
        assert_eq!(
            points.first().copied(),
            Some(widget.display_curve_point(bounds, left))
        );
        assert_eq!(
            points.last().copied(),
            Some(widget.display_curve_point(bounds, right))
        );
    }

    #[test]
    fn curve_preview_widget_paints_subtle_fill_beneath_curve() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let fill = primitives
            .iter()
            .find_map(|primitive| match primitive {
                PaintPrimitive::FillPath(fill) => Some(fill),
                _ => None,
            })
            .expect("curve preview should emit one gradient attenuation fill path");
        assert_eq!(
            fill.path.commands().len(),
            widget.sample_curve_points(bounds).len() + 3
        );
        assert_eq!(
            fill.brush,
            PaintBrush::linear_gradient(PaintLinearGradient::vertical(
                CurvePreviewWidget::curve_bounds(bounds),
                theme.accent_mint.with_alpha(CURVE_FILL_TOP_ALPHA),
                theme.accent_mint.with_alpha(CURVE_FILL_BOTTOM_ALPHA),
            ))
        );
    }

    #[test]
    fn curve_preview_widget_grid_tracks_sync_length_and_resize_geometry() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let theme = ThemeTokens::default();
        let grid_x_positions = |width: f32| {
            let widget =
                CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false)
                    .with_sync_division(7);
            let bounds = Rect::from_xy_size(0.0, 0.0, width, CURVE_PREVIEW_HEIGHT);
            let mut primitives = Vec::new();
            widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);
            let curve_bounds = CurvePreviewWidget::curve_bounds(bounds);
            let positions = primitives
                .iter()
                .filter_map(|primitive| match primitive {
                    PaintPrimitive::StrokePolyline(stroke)
                        if stroke.color == theme.grid_strong
                            && stroke.points.len() == 2
                            && (stroke.points[0].x - stroke.points[1].x).abs() < 1.0e-6 =>
                    {
                        Some(stroke.points[0].x)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            (curve_bounds, positions)
        };

        let (wide_bounds, wide) = grid_x_positions(396.0);
        let (narrow_bounds, narrow) = grid_x_positions(198.0);
        assert_eq!(wide.len(), 7);
        assert_eq!(narrow.len(), 7);
        for (wide_x, narrow_x) in wide.iter().zip(narrow.iter()) {
            let wide_normalized = (*wide_x - wide_bounds.min.x) / (wide_bounds.width() - 1.0);
            let narrow_normalized =
                (*narrow_x - narrow_bounds.min.x) / (narrow_bounds.width() - 1.0);
            assert!((wide_normalized - narrow_normalized).abs() < 1.0e-4);
        }
    }

    #[test]
    fn curve_preview_widget_gain_references_track_curve_mapping_and_stay_labeled() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let theme = ThemeTokens::default();

        for bounds in [
            Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT),
            Rect::from_xy_size(0.0, 0.0, 270.0, 51.0),
        ] {
            let widget =
                CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false);
            let mut primitives = Vec::new();
            widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

            let guides: Vec<_> = primitives
                .iter()
                .filter_map(|primitive| match primitive {
                    PaintPrimitive::StrokePolyline(stroke)
                        if stroke.color == theme.text_muted.with_alpha(72)
                            && stroke.points.len() == 2
                            && (stroke.points[0].y - stroke.points[1].y).abs() < 1.0e-6 =>
                    {
                        Some((stroke.points[0].y, stroke.points[0].x, stroke.points[1].x))
                    }
                    _ => None,
                })
                .collect();
            let guide_y_positions: Vec<_> = guides.iter().map(|(y, _, _)| *y).collect();
            let curve_bounds = CurvePreviewWidget::curve_bounds(bounds);
            let expected_y_positions: Vec<_> = super::super::curve_gain_references()
                .iter()
                .map(|reference| {
                    CurvePreviewWidget::curve_point(
                        bounds,
                        CurveNode {
                            x: 0.0,
                            y: reference.gain,
                        },
                    )
                    .y
                })
                .collect();
            let labels: Vec<_> = primitives
                .iter()
                .filter_map(|primitive| match primitive {
                    PaintPrimitive::Text(text)
                        if ["0 dB", "−6 dB", "−12 dB", "−∞"].contains(&text.text.as_str()) =>
                    {
                        assert!(text.rect.min.x >= bounds.min.x);
                        assert!(text.rect.min.y >= bounds.min.y);
                        assert!(text.rect.max.x <= bounds.max.x);
                        assert!(text.rect.max.y <= bounds.max.y);
                        assert!(text.rect.max.x <= curve_bounds.min.x);
                        Some(text.text.as_str())
                    }
                    _ => None,
                })
                .collect();

            assert_eq!(guide_y_positions, expected_y_positions);
            assert!(guides.iter().all(|(_, start_x, end_x)| {
                (*start_x - curve_bounds.min.x).abs() < 1.0e-6
                    && (*end_x - curve_bounds.max.x).abs() < 1.0e-6
            }));
            assert_eq!(labels, ["0 dB", "−6 dB", "−12 dB", "−∞"]);
            assert!((guide_y_positions[0] - bounds.min.y).abs() < 1.0e-6);
            assert!((guide_y_positions[3] - (curve_bounds.max.y - 1.0)).abs() < 1.0e-6);
        }
    }

    #[test]
    fn curve_preview_widget_maps_processed_waveform_with_display_phase_and_gain() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.0 },
                CurveNode { x: 0.3, y: 0.8 },
                CurveNode { x: 0.7, y: 0.1 },
                CurveNode { x: 1.0, y: 0.2 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 3],
            ..EditableCurve::default()
        }
        .normalized();
        let mut waveform = [0.0; crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT];
        waveform[0] = 0.8;
        waveform[48] = 0.9;
        let phase_offset = 0.25;
        let widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false)
            .with_incoming_waveform(Some(waveform))
            .with_phase_offset(phase_offset)
            .with_gain_mapping(120.0, -60.0);

        let processed = widget
            .processed_waveform()
            .expect("incoming waveform should produce a wet preview");
        assert!(
            (processed[0] - waveform[0] * sample_editable_curve(&curve, -phase_offset)).abs()
                < 1.0e-6
        );
        assert!(
            (processed[48]
                - waveform[48]
                    * sample_editable_curve(
                        &curve,
                        48.0 / (waveform.len() - 1) as f32 - phase_offset,
                    ))
            .abs()
                < 1.0e-6
        );
    }

    #[test]
    fn curve_preview_widget_processed_waveform_preserves_unity_and_zero_gain() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.0 },
                CurveNode { x: 0.5, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 2],
            ..EditableCurve::default()
        }
        .normalized();
        let waveform = [0.73; crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT];
        let unity = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false)
            .with_incoming_waveform(Some(waveform))
            .with_gain_mapping(0.0, -60.0)
            .processed_waveform()
            .expect("incoming waveform should produce a wet preview");
        assert!(unity.iter().all(|sample| (*sample - 0.73).abs() < 1.0e-6));

        let zero = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_incoming_waveform(Some(waveform))
            .with_gain_mapping(120.0, -60.0)
            .processed_waveform()
            .expect("incoming waveform should produce a wet preview");
        assert!(zero[0].abs() < 1.0e-6);
        assert!(zero[48] > 0.0);
    }

    #[test]
    fn curve_preview_widget_processed_waveform_clamps_finite_floor_and_input() {
        let curve = EditableCurve {
            nodes: vec![
                CurveNode { x: 0.0, y: 0.0 },
                CurveNode { x: 0.5, y: 0.5 },
                CurveNode { x: 1.0, y: 1.0 },
            ],
            segments: vec![CurveSegment { tension: 0.0 }; 2],
            ..EditableCurve::default()
        }
        .normalized();
        let mut waveform = [0.25; crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT];
        waveform[1] = 1.5;
        waveform[2] = -0.5;
        waveform[3] = f32::NAN;

        let finite_floor =
            CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false)
                .with_incoming_waveform(Some(waveform))
                .with_gain_mapping(120.0, -6.0)
                .processed_waveform()
                .expect("incoming waveform should produce a wet preview");
        assert!((finite_floor[0] - 0.25 * crate::dsp::db_to_linear(-6.0)).abs() < 1.0e-6);

        let clamped_input = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_incoming_waveform(Some(waveform))
            .with_gain_mapping(0.0, 0.0)
            .processed_waveform()
            .expect("incoming waveform should produce a wet preview");
        assert_eq!(clamped_input[1], 1.0);
        assert_eq!(clamped_input[2], 0.0);
        assert_eq!(clamped_input[3], 0.0);
        assert!(clamped_input.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn curve_preview_widget_paints_waveform_layers_with_shared_geometry_and_z_order() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let mut waveform = [0.0; crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT];
        waveform[crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT / 2] = 1.0;
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_incoming_waveform(Some(waveform));
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let processed_color = theme.accent_copper.with_alpha(96);
        let incoming_color = theme.text_muted.with_alpha(88);
        let processed_indices: Vec<_> = primitives
            .iter()
            .enumerate()
            .filter_map(|(index, primitive)| match primitive {
                PaintPrimitive::StrokePolyline(stroke)
                    if stroke.color == processed_color && (stroke.width - 2.0).abs() < 1.0e-6 =>
                {
                    Some(index)
                }
                _ => None,
            })
            .collect();
        let incoming_indices: Vec<_> = primitives
            .iter()
            .enumerate()
            .filter_map(|(index, primitive)| match primitive {
                PaintPrimitive::StrokePolyline(stroke)
                    if stroke.color == incoming_color && (stroke.width - 1.0).abs() < 1.0e-6 =>
                {
                    Some(index)
                }
                _ => None,
            })
            .collect();
        assert_eq!(processed_indices.len(), 2);
        assert_eq!(incoming_indices.len(), 2);
        for (processed_index, incoming_index) in
            processed_indices.iter().zip(incoming_indices.iter())
        {
            let processed = match &primitives[*processed_index] {
                PaintPrimitive::StrokePolyline(stroke) => stroke,
                _ => unreachable!(),
            };
            let incoming = match &primitives[*incoming_index] {
                PaintPrimitive::StrokePolyline(stroke) => stroke,
                _ => unreachable!(),
            };
            assert_eq!(processed.points.len(), waveform.len());
            assert_eq!(processed.points.len(), incoming.points.len());
            let curve_bounds = CurvePreviewWidget::curve_bounds(bounds);
            let endpoint_scale = curve_bounds.width().max(1.0) - 1.0;
            assert!((processed.points[0].x - curve_bounds.min.x).abs() < 1.0e-6);
            assert!((incoming.points[0].x - curve_bounds.min.x).abs() < 1.0e-6);
            assert!(
                (processed.points[processed.points.len() - 1].x
                    - (curve_bounds.min.x + endpoint_scale))
                    .abs()
                    < 1.0e-6
            );
            assert!(
                (incoming.points[incoming.points.len() - 1].x
                    - (curve_bounds.min.x + endpoint_scale))
                    .abs()
                    < 1.0e-6
            );
            assert!(processed
                .points
                .iter()
                .zip(incoming.points.iter())
                .all(|(processed, incoming)| (processed.x - incoming.x).abs() < 1.0e-6));
        }
        let curve_index = primitives
            .iter()
            .position(|primitive| {
                matches!(
                    primitive,
                    PaintPrimitive::StrokePolyline(stroke)
                        if stroke.color == theme.accent_mint && (stroke.width - 1.7).abs() < 1.0e-6
                )
            })
            .expect("editable curve stroke should be present");
        let guide_index = primitives
            .iter()
            .position(|primitive| {
                matches!(
                    primitive,
                    PaintPrimitive::StrokePolyline(stroke)
                        if stroke.color == theme.text_muted.with_alpha(72)
                )
            })
            .expect("curve gain guide should be present");
        let node_index = primitives
            .iter()
            .rposition(|primitive| {
                matches!(
                    primitive,
                    PaintPrimitive::FillRect(fill) if fill.color == theme.surface_raised
                )
            })
            .expect("curve nodes should be present");
        assert!(processed_indices.iter().all(|index| *index < curve_index));
        assert!(incoming_indices.iter().all(|index| *index < curve_index));
        assert!(incoming_indices.iter().all(|index| *index < guide_index));
        assert!(guide_index < curve_index);
        assert!(processed_indices
            .iter()
            .chain(incoming_indices.iter())
            .all(|index| *index < node_index));
        assert!(processed_indices
            .iter()
            .all(|index| *index < incoming_indices[0]));
    }

    #[test]
    fn curve_preview_widget_paints_hovered_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, None, None, Some(1), None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::FillRect(fill)
                    if fill.color == theme.accent_mint
                        && (fill.rect.width() - (CURVE_NODE_SIZE + 1.275)).abs() < 1.0e-5
                        && (fill.rect.height() - (CURVE_NODE_SIZE + 1.275)).abs() < 1.0e-5
            )
        }));
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokeRect(stroke)
                    if stroke.color == theme.accent_warning
                        && (stroke.rect.width() - (CURVE_NODE_SIZE + 1.275)).abs() < 1.0e-5
                        && (stroke.width - 1.275).abs() < 1.0e-6
            )
        }));
    }

    #[test]
    fn curve_preview_widget_paints_selected_nodes_and_active_marquee() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let start = CurveNode { x: 0.12, y: 0.94 };
        let current = CurveNode { x: 0.64, y: 0.16 };
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_selected_curve_nodes(&[1])
            .with_active_curve_marquee(Some(ActiveCurveMarquee { start, current }));
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let selected_size = CURVE_NODE_SIZE + 1.7;
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::FillRect(fill)
                    if fill.color == theme.accent_warning
                        && (fill.rect.width() - selected_size).abs() < 1.0e-5
            )
        }));
        let start_point = CurvePreviewWidget::curve_point(bounds, start);
        let current_point = CurvePreviewWidget::curve_point(bounds, current);
        let expected_rect = Rect::from_xy_size(
            start_point.x.min(current_point.x),
            start_point.y.min(current_point.y),
            (start_point.x - current_point.x).abs(),
            (start_point.y - current_point.y).abs(),
        );
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokeRect(stroke)
                    if stroke.color == theme.accent_mint
                        && stroke.rect == expected_rect
            )
        }));
    }

    #[test]
    fn gain_reduction_meter_paints_target_width_segments_and_labels() {
        let bounds = Rect::from_xy_size(0.0, 0.0, GAIN_REDUCTION_METER_WIDTH, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut unity_primitives = Vec::new();
        GainReductionMeterWidget::new(0.0).append_paint(
            &mut unity_primitives,
            bounds,
            &LayoutOutput::default(),
            &theme,
        );
        let meter_colors = pump_meter_colors();
        assert!(!unity_primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::FillRect(fill) if fill.color == meter_colors.nominal)
        }));

        let reduction_db = crate::gui_status::GAIN_REDUCTION_METER_MAX_DB * 0.5;
        let mut reduced_primitives = Vec::new();
        GainReductionMeterWidget::new(reduction_db).append_paint(
            &mut reduced_primitives,
            bounds,
            &LayoutOutput::default(),
            &theme,
        );
        let fills: Vec<_> = reduced_primitives
            .iter()
            .filter_map(|primitive| match primitive {
                PaintPrimitive::FillRect(fill) if fill.color == meter_colors.nominal => Some(fill),
                _ => None,
            })
            .collect();
        assert!(
            !fills.is_empty(),
            "reduction should paint active meter segments"
        );
        assert!(fills.iter().all(|fill| {
            (fill.rect.width() - (GAIN_REDUCTION_METER_BAR_WIDTH - 2.0)).abs() < 1.0e-5
                && (fill.rect.height() - PUMP_VISUAL_METRICS.meter_segment).abs() < 1.0e-5
        }));
        assert!(
            (fills[0].rect.min.y - (bounds.min.y + PUMP_TYPOGRAPHY.meta.1 + 1.0)).abs() < 1.0e-6
        );
        for expected in ["GR dB", "18.0"] {
            assert!(reduced_primitives.iter().any(|primitive| {
                matches!(primitive, PaintPrimitive::Text(text) if text.text.as_str() == expected)
            }));
        }
    }

    #[test]
    fn curve_preview_widget_paints_hovered_active_node_outline() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, Some(1), None, Some(1), None, None, false);
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::FillRect(fill)
                    if fill.color == theme.accent_warning
                        && (fill.rect.width() - (CURVE_NODE_SIZE + 1.7)).abs() < 1.0e-5
                        && (fill.rect.height() - (CURVE_NODE_SIZE + 1.7)).abs() < 1.0e-5
            )
        }));
        assert!(primitives.iter().any(|primitive| {
            matches!(
                primitive,
                PaintPrimitive::StrokeRect(stroke)
                    if stroke.color == theme.accent_mint
                        && (stroke.rect.width() - (CURVE_NODE_SIZE + 1.7)).abs() < 1.0e-5
                        && (stroke.width - 1.275).abs() < 1.0e-6
            )
        }));
    }

    #[test]
    fn curve_preview_widget_paints_playhead_marker_at_sampled_phase() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let phase = 0.37;
        let widget = CurvePreviewWidget::new(curve.clone(), None, None, None, None, None, false)
            .with_playhead_phase(Some(phase));
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let expected_center = CurvePreviewWidget::curve_point(
            bounds,
            CurveNode {
                x: phase,
                y: sample_editable_curve(&curve, phase).clamp(0.0, 1.0),
            },
        );
        assert!(primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::StrokePolyline(line)
                if line.color == CURVE_PLAYHEAD_CORE_COLOR
                    && line.points.len() == 2
                    && (line.points[0].x - expected_center.x).abs() < 1.0e-4
                    && (line.points[1].x - expected_center.x).abs() < 1.0e-4)
        }));
    }

    #[test]
    fn curve_preview_widget_paints_playhead_above_overlapping_node() {
        let curve = PumpParams::new().editable_curve_snapshot();
        let widget = CurvePreviewWidget::new(curve, None, None, None, None, None, false)
            .with_playhead_phase(Some(0.0));
        let bounds = Rect::from_xy_size(0.0, 0.0, 396.0, CURVE_PREVIEW_HEIGHT);
        let theme = ThemeTokens::default();
        let mut primitives = Vec::new();

        widget.append_paint(&mut primitives, bounds, &LayoutOutput::default(), &theme);

        let endpoint_node_index = primitives
            .iter()
            .enumerate()
            .filter_map(|(index, primitive)| match primitive {
                PaintPrimitive::StrokeRect(stroke)
                    if stroke.color == theme.accent_copper
                        && (stroke.rect.width() - CURVE_NODE_SIZE).abs() < 1.0e-6
                        && (stroke.rect.height() - CURVE_NODE_SIZE).abs() < 1.0e-6 =>
                {
                    Some(index)
                }
                _ => None,
            })
            .next()
            .expect("default curve should paint its endpoint node");
        let playhead_core_index = primitives
            .iter()
            .position(|primitive| {
                matches!(
                    primitive,
                    PaintPrimitive::StrokePolyline(line)
                        if line.color == CURVE_PLAYHEAD_CORE_COLOR && line.points.len() == 2
                )
            })
            .expect("phase-zero playhead should paint over the endpoint node");

        assert!(
            playhead_core_index > endpoint_node_index,
            "playhead must be appended after overlapping node primitives"
        );
    }

    #[test]
    fn curve_playhead_palette_does_not_reuse_editable_node_states() {
        let theme = ThemeTokens::default();
        let editable_node_colors = [
            theme.surface_raised,
            theme.accent_warning,
            theme.accent_mint,
            theme.accent_copper,
        ];

        for editable_color in editable_node_colors {
            assert_ne!(CURVE_PLAYHEAD_CORE_COLOR, editable_color);
        }
    }

    #[cfg(feature = "vst3")]
    #[test]
    fn radiant_editor_consumes_inactive_meter_clear_repaint() {
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        let mut editor = RadiantPumpEditor::new(
            params,
            Arc::clone(&status),
            Arc::new(PumpAutomationQueue::default()),
            None,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );
        status.update(
            0.0,
            0.25,
            GuiTransportTelemetry {
                is_playing: true,
                transport_is_playing: true,
                has_host_beats_timeline: false,
                beat_phase: 0.0,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        assert!(status.gain_reduction_db() > 0.0);
        status.update_transport(
            0.0,
            GuiTransportTelemetry {
                is_playing: false,
                transport_is_playing: false,
                has_host_beats_timeline: false,
                beat_phase: 0.0,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        status.mark_gain_reduction_inactive();

        assert!(editor.needs_realtime_redraw());
        let _ = editor.paint_plan();
        assert!(
            !editor.needs_realtime_redraw(),
            "painting zero must consume the one-shot clear repaint"
        );
    }

    #[cfg(feature = "vst3")]
    #[test]
    fn radiant_editor_reprojects_playhead_from_status_without_pointer_event() {
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        status.update(
            0.1,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                transport_is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.1,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        let mut editor = RadiantPumpEditor::new(
            Arc::clone(&params),
            Arc::clone(&status),
            Arc::new(PumpAutomationQueue::default()),
            None,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );
        let first_center = playhead_marker_center(editor.paint_plan())
            .expect("initial paint plan should include a playhead marker");

        status.update(
            0.6,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                transport_is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.6,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        let second_center = playhead_marker_center(editor.paint_plan())
            .expect("refreshed paint plan should include a playhead marker");

        assert!(
            (second_center.x - first_center.x).abs() > 32.0,
            "playhead marker should move after status changes without relying on pointer events (first={first_center:?}, second={second_center:?})"
        );
    }

    #[cfg(feature = "vst3")]
    fn playhead_marker_center(plan: &SurfacePaintPlan) -> Option<Point> {
        plan.primitives
            .iter()
            .find_map(|primitive| match primitive {
                PaintPrimitive::StrokePolyline(line)
                    if line.color == CURVE_PLAYHEAD_CORE_COLOR && line.points.len() == 2 =>
                {
                    Some(Point::new(line.points[0].x, line.points[0].y))
                }
                _ => None,
            })
    }

    #[test]
    fn radiant_editor_surface_emits_visible_paint() {
        let frame = radiant_editor_frame_for_params(
            {
                let params = PumpParams::new();
                params.set_smooth(0.67);
                Arc::new(params)
            },
            Arc::new(GuiStatus::default()),
            Vector2::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
        );
        let version_label = build_version_label();

        assert_eq!(
            frame
                .paint_plan
                .primitives
                .iter()
                .filter(|primitive| {
                    matches!(primitive, PaintPrimitive::Text(text) if text.text.as_str() == version_label)
                })
                .count(),
            1,
            "header should paint one subtle build label"
        );
        let pump_labels: Vec<_> = frame
            .paint_plan
            .primitives
            .iter()
            .filter_map(|primitive| match primitive {
                PaintPrimitive::Text(text) if text.text == "PUMP" => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(pump_labels.len(), 1, "header should paint one wordmark");
        assert_eq!(pump_labels[0].align, PaintTextAlign::Right);
        let brand_labels: Vec<_> = frame
            .paint_plan
            .primitives
            .iter()
            .filter_map(|primitive| match primitive {
                PaintPrimitive::Text(text)
                    if matches!(text.text.as_str(), "PORTALSURFER" | "/" | "PUMP") =>
                {
                    Some(text)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            brand_labels.len(),
            3,
            "header should paint one complete brand"
        );
        let [vendor, separator, wordmark] = brand_labels.as_slice() else {
            unreachable!("header brand should contain exactly three text cells")
        };
        assert_eq!(vendor.rect.width(), HEADER_BRAND_WORDMARK_WIDTH);
        assert!(
            vendor.rect.max.x <= separator.rect.min.x
                && separator.rect.max.x <= wordmark.rect.min.x,
            "header brand cells must be ordered and non-overlapping"
        );
        assert!(
            frame.paint_plan.primitives.iter().any(
                |primitive| matches!(primitive, PaintPrimitive::Text(text) if text.text == "?")
            ),
            "header should paint the clickable hotkey help control"
        );
        assert!(
            frame
                .paint_plan
                .primitives
                .iter()
                .filter(|primitive| matches!(primitive, PaintPrimitive::Svg(_)))
                .count()
                >= 3,
            "editor action buttons must paint retained SVG icons"
        );
        for obsolete_label in ["<", ">", "F", "+", "S", "Undo", "Redo"] {
            assert!(
                frame
                    .paint_plan
                    .primitives
                    .iter()
                    .all(|primitive| !matches!(primitive, PaintPrimitive::Text(text)
                        if text.text.as_str() == obsolete_label)),
                "{obsolete_label} must not remain as action text paint"
            );
        }
        assert!(frame
            .paint_plan
            .primitives
            .iter()
            .any(|primitive| matches!(primitive, PaintPrimitive::FillRect(_))));
        assert!(frame.paint_plan.primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::StrokePolyline(polyline) if polyline.points.len() > 16)
        }));
    }

    #[test]
    fn sync_dropdown_overlay_is_below_trigger_above_curve_and_selectable() {
        assert_eq!(TIMING_MODE_TOGGLE_WIDTH, 54.4);
        assert_eq!(TIMING_CONTROL_HEIGHT, 34.0);
        assert_eq!(TIMING_DROPDOWN_WIDTH, 95.2);
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(params);
        state.timing_dropdown_open = true;

        let frame = project_editor_surface(&mut state).frame_at_size(
            Vector2::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
            &pump_theme(),
        );
        let option_entries: Vec<_> = frame
            .paint_plan
            .primitives
            .iter()
            .enumerate()
            .filter_map(|(index, primitive)| match primitive {
                PaintPrimitive::Text(text)
                    if SYNC_DIVISIONS
                        .iter()
                        .any(|division| division.label == text.text.as_str()) =>
                {
                    Some((index, text.rect))
                }
                _ => None,
            })
            .collect();

        assert_eq!(
            option_entries.len(),
            SYNC_DIVISIONS.len(),
            "open dropdown should paint every sync option"
        );
        assert!(frame.paint_plan.primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::Text(text) if text.text.as_str() == "Sync 1/4")
        }));
        let mut menu_entries = option_entries.iter();
        let curve_index = frame
            .paint_plan
            .primitives
            .iter()
            .rposition(|primitive| {
                matches!(
                    primitive,
                    PaintPrimitive::StrokePolyline(polyline) if polyline.points.len() > 16
                )
            })
            .expect("curve canvas should paint a sampled curve polyline");
        let timing_trigger_left =
            SURFACE_PADDING + TIMING_MODE_TOGGLE_WIDTH + PUMP_VISUAL_METRICS.space_4;
        let timing_bottom = SURFACE_PADDING + TIMING_CONTROL_HEIGHT;
        assert!(
            menu_entries.all(|(index, rect)| {
                *index > curve_index
                    && rect.min.x >= timing_trigger_left
                    && rect.min.y >= timing_bottom
            }),
            "sync options should align below the selector trigger, above the curve canvas"
        );

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::SyncDivision(normalize_sync_division(6)),
        );
        assert_eq!(state.params.sync_division(), 6);
        assert!(
            !state.timing_dropdown_open,
            "selecting an option should close the menu"
        );
    }

    #[test]
    fn hotkey_help_overlay_paints_documented_shortcuts() {
        let params = Arc::new(PumpParams::new());
        let mut state = editor_state(params);
        state.hotkey_help_open = true;

        let frame = project_editor_surface(&mut state).frame_at_size(
            Vector2::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
            &pump_theme(),
        );
        for label in [
            "PUMP HOTKEYS",
            "u",
            "U",
            "Shift + drag node",
            "Shift + Option + drag node",
            "Cmd + drag node",
            "Shift + drag canvas",
            "Option + drag segment",
            "Cmd + drag segment",
            "Cmd + Shift + drag canvas",
            "Cmd + Shift + Option + drag canvas",
        ] {
            assert!(
                frame.paint_plan.primitives.iter().any(|primitive| matches!(
                    primitive,
                    PaintPrimitive::Text(text) if text.text.as_str() == label
                )),
                "hotkey help must paint {label}"
            );
        }
    }

    #[test]
    fn timing_toggle_switches_modes_and_free_unit_selection_is_sound_neutral() {
        let params = Arc::new(PumpParams::new());
        params.set_free_rate_hz(8.0);
        let mut state = editor_state(params);

        reduce_editor_message(&mut state, RadiantEditorMessage::ToggleTimingMode);
        assert_eq!(state.params.timing_mode(), TIMING_MODE_FREE);
        assert_eq!(state.params.sync_division(), 4);
        assert_eq!(state.free_rate_unit, FreeRateUnit::Hertz);

        state.timing_dropdown_open = true;
        let frame = project_editor_surface(&mut state).frame_at_size(
            Vector2::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
            &pump_theme(),
        );
        for unit in FreeRateUnit::ALL {
            assert!(frame.paint_plan.primitives.iter().any(|primitive| {
                matches!(primitive, PaintPrimitive::Text(text) if text.text.as_str() == unit.label())
            }));
        }

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::FreeRateUnit(FreeRateUnit::Milliseconds),
        );
        assert_eq!(state.free_rate_unit, FreeRateUnit::Milliseconds);
        assert!(!state.timing_dropdown_open);
        assert!((state.params.free_rate_hz() - 8.0).abs() < f32::EPSILON);
    }

    #[test]
    fn sync_dropdown_dismisses_before_interactive_curve_click() {
        let params = Arc::new(PumpParams::new());
        let status = Arc::new(GuiStatus::default());
        let mut editor = RadiantPumpEditor::new(
            Arc::clone(&params),
            status,
            Arc::new(PumpAutomationQueue::default()),
            None,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );
        editor.runtime.bridge_mut().state_mut().timing_dropdown_open = true;
        editor.runtime.refresh();

        let origin = params.editable_curve_snapshot();
        let position = Point::new(300.0, 220.0);
        editor.dispatch_event(radiant::runtime::Event::pointer_press(
            position,
            PointerButton::Primary,
            PointerModifiers::default(),
        ));
        editor.dispatch_event(radiant::runtime::Event::pointer_release(
            position,
            PointerButton::Primary,
            PointerModifiers::default(),
        ));

        assert!(
            !editor.runtime.bridge().state().timing_dropdown_open,
            "an outside click should dismiss the dropdown"
        );
        assert_eq!(
            params.editable_curve_snapshot(),
            origin,
            "dismissing on a curve click must not edit the underlying curve"
        );
    }

    #[test]
    fn parameter_deck_hides_legacy_depth_and_floor_controls() {
        let params = PumpParams::new();
        params.set_smooth(0.67);
        let frame = radiant_editor_frame_for_params(
            Arc::new(params),
            Arc::new(GuiStatus::default()),
            Vector2::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
        );
        let labels: Vec<_> = frame
            .paint_plan
            .primitives
            .iter()
            .filter_map(|primitive| match primitive {
                PaintPrimitive::Text(text)
                    if matches!(
                        text.text.as_str(),
                        "DEPTH" | "FLOOR" | "OFFSET" | "SMOOTH" | "MIX" | "OUTPUT"
                    ) =>
                {
                    Some(text.text.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(labels, ["OFFSET", "SMOOTH", "MIX", "OUTPUT"]);
        assert!(frame.paint_plan.primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::Text(text) if text.text.as_str() == "67%")
        }));
    }

    #[test]
    fn knob_routes_pointer_keyboard_and_reset_lifecycles_without_batch_spam() {
        let params = Arc::new(PumpParams::new());
        let queue = Arc::new(PumpAutomationQueue::default());
        let mut state = RadiantEditorState::new(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Arc::new(ClapHostParamEditSink {
                queue: Arc::clone(&queue),
                requester: None,
            }),
        );

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Knob {
                target: NumericEntryTarget::FreeRate,
                message: KnobMessage::GestureStarted { value: 0.0 },
            },
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Knob {
                target: NumericEntryTarget::FreeRate,
                message: KnobMessage::ValueChanged { value: 0.25 },
            },
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Knob {
                target: NumericEntryTarget::FreeRate,
                message: KnobMessage::ValueChanged { value: 0.5 },
            },
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Knob {
                target: NumericEntryTarget::FreeRate,
                message: KnobMessage::GestureEnded { value: 0.5 },
            },
        );
        assert!((params.free_rate_hz() - 31.622_776).abs() < 1.0e-3);

        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        let stats = queue.drain_to_output(&mut output, &mut scratch);
        assert_eq!(stats.attempted, 4);
        let kinds: Vec<_> = (0..buffer.len())
            .filter_map(|index| {
                let event = buffer.get(index as u32)?;
                Some(match event.as_core_event() {
                    Some(CoreEventSpace::ParamValue(value)) => {
                        assert_eq!(value.param_id(), Some(PARAM_FREE_RATE_ID));
                        "value"
                    }
                    Some(CoreEventSpace::ParamGestureBegin(_)) => "begin",
                    Some(CoreEventSpace::ParamGestureEnd(_)) => "end",
                    _ => "other",
                })
            })
            .collect();
        assert_eq!(kinds, ["other", "value", "value", "other"]);

        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Knob {
                target: NumericEntryTarget::FreeRate,
                message: KnobMessage::KeyboardGesture(radiant::prelude::KnobKeyboardGesture::new(
                    0.5, 0.75,
                )),
            },
        );
        reduce_editor_message(
            &mut state,
            RadiantEditorMessage::Knob {
                target: NumericEntryTarget::FreeRate,
                message: KnobMessage::Reset { value: 0.0 },
            },
        );
        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        assert_eq!(
            queue.drain_to_output(&mut output, &mut scratch).attempted,
            6
        );
        assert!((params.free_rate_hz() - 0.05).abs() < f32::EPSILON);
    }

    #[test]
    fn clap_sink_enqueues_each_continuous_event_as_it_arrives() {
        let queue = Arc::new(PumpAutomationQueue::default());
        let sink = ClapHostParamEditSink {
            queue: Arc::clone(&queue),
            requester: None,
        };
        let config = AutomationConfig::default();

        assert!(sink.gesture_started(&config, PARAM_FREE_RATE_ID));
        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        assert_eq!(
            queue.drain_to_output(&mut output, &mut scratch).attempted,
            1
        );

        assert!(sink.gesture_value(&config, PARAM_FREE_RATE_ID, 0.5));
        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        assert_eq!(
            queue.drain_to_output(&mut output, &mut scratch).attempted,
            1
        );

        assert!(sink.gesture_ended(&config, PARAM_FREE_RATE_ID));
        let mut buffer = EventBuffer::new();
        let mut output = buffer.as_output();
        let mut scratch = Vec::new();
        assert_eq!(
            queue.drain_to_output(&mut output, &mut scratch).attempted,
            1
        );
    }

    #[test]
    fn clap_sink_preserves_lifecycle_order_for_each_deck_control() {
        let config = AutomationConfig::default();
        for param_id in [
            PARAM_SMOOTH_ID,
            PARAM_MIX_ID,
            PARAM_OUTPUT_GAIN_ID,
            PARAM_FREE_RATE_ID,
        ] {
            let queue = Arc::new(PumpAutomationQueue::default());
            let sink = ClapHostParamEditSink {
                queue: Arc::clone(&queue),
                requester: None,
            };
            assert!(sink.gesture_started(&config, param_id));
            assert!(sink.gesture_value(&config, param_id, 0.25));
            assert!(sink.gesture_ended(&config, param_id));

            let mut buffer = EventBuffer::new();
            let mut output = buffer.as_output();
            let mut scratch = Vec::new();
            assert_eq!(
                queue.drain_to_output(&mut output, &mut scratch).attempted,
                3,
                "each deck control should emit one complete lifecycle"
            );
            let kinds: Vec<_> = (0..buffer.len())
                .map(|index| {
                    let event = buffer
                        .get(index as u32)
                        .expect("lifecycle event should be present");
                    match event.header().type_id() {
                        ParamGestureBeginEvent::TYPE_ID => {
                            assert_eq!(
                                event
                                    .as_event::<ParamGestureBeginEvent>()
                                    .expect("gesture begin should decode")
                                    .param_id(),
                                Some(param_id)
                            );
                            "begin"
                        }
                        ParamValueEvent::TYPE_ID => {
                            let CoreEventSpace::ParamValue(event) = event
                                .as_core_event()
                                .expect("value should decode as a core event")
                            else {
                                unreachable!()
                            };
                            assert_eq!(event.param_id(), Some(param_id));
                            "value"
                        }
                        ParamGestureEndEvent::TYPE_ID => {
                            assert_eq!(
                                event
                                    .as_event::<ParamGestureEndEvent>()
                                    .expect("gesture end should decode")
                                    .param_id(),
                                Some(param_id)
                            );
                            "end"
                        }
                        _ => "other",
                    }
                })
                .collect();
            assert_eq!(kinds, ["begin", "value", "end"]);
        }
    }

    #[test]
    fn radiant_editor_does_not_paint_bottom_ab_status_text() {
        let params = Arc::new(PumpParams::new());
        params.set_mix(0.25);
        let frame = radiant_editor_frame_for_params(
            Arc::clone(&params),
            Arc::new(GuiStatus::default()),
            Vector2::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
        );
        assert!(frame.paint_plan.primitives.iter().all(|primitive| {
            !matches!(primitive, PaintPrimitive::Text(text) if text.text.contains("A active · A/B"))
        }));
        params.copy_active_to_inactive();
        let frame = radiant_editor_frame_for_params(
            params,
            Arc::new(GuiStatus::default()),
            Vector2::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
        );
        assert!(frame.paint_plan.primitives.iter().all(|primitive| {
            !matches!(primitive, PaintPrimitive::Text(text) if text.text.contains("A active · A/B"))
        }));
    }
}
