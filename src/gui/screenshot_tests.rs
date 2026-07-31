//! Renderer-parity screenshots for the macOS Radiant editor.

use std::{fs, path::PathBuf, sync::Arc};

use crate::automation_queue::PumpAutomationQueue;
use image::{ColorType, ImageFormat};
use radiant::{
    gui::{
        svg::IconName,
        types::{Point, Rect, Rgba8, Vector2},
    },
    layout::LayoutOutput,
    runtime::{
        Event, PaintFillRect, PaintPathCommand, PaintPrimitive, PaintStrokePolyline, PaintText,
        PaintTextAlign, PaintTextRun, SurfacePaintPlan,
    },
    theme::DpiScale,
    widgets::{
        ButtonWidget, CardWidget, IconButtonWidget, KnobWidget, TextWrap, Widget, WidgetSizing,
        WidgetState,
    },
};

use super::visual_system::{pump_meter_colors, pump_theme, PUMP_TYPOGRAPHY, PUMP_VISUAL_METRICS};
use super::{
    RadiantPumpEditor, MAX_WINDOW_HEIGHT, MAX_WINDOW_WIDTH, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
    WINDOW_HEIGHT, WINDOW_WIDTH,
};
use crate::{
    curve_presets::quick_slot_seeds,
    params::{sync_division_label, with_test_curve_slot_path, PumpParams, SoundSide},
    GuiStatus,
};

fn screenshot_root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("ui-screenshots")
        .join("pump");
    fs::create_dir_all(&root).expect("screenshot directory should be writable");
    root
}

fn render_case(name: &str, width: u32, height: u32, dpi: DpiScale) -> (SurfacePaintPlan, Vec<u8>) {
    render_case_with_bypass(name, width, height, dpi, false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeaderCaptureState {
    Normal,
    Hovered,
    CopyHovered,
    AHovered,
    BHovered,
    Pressed,
    Disabled,
    AActive,
    BActive,
}

fn header_svg_centers(plan: &SurfacePaintPlan) -> Vec<Point> {
    plan.primitives
        .iter()
        .filter_map(|primitive| match primitive {
            PaintPrimitive::Svg(svg) if svg.rect.min.y < 70.0 => Some(Point::new(
                svg.rect.min.x + svg.rect.width() * 0.5,
                svg.rect.min.y + svg.rect.height() * 0.5,
            )),
            _ => None,
        })
        .collect()
}

fn header_text_center(plan: &SurfacePaintPlan, label: &str) -> Point {
    plan.primitives
        .iter()
        .find_map(|primitive| match primitive {
            PaintPrimitive::Text(text) if text.text.as_str() == label && text.rect.min.y < 70.0 => {
                Some(Point::new(
                    text.rect.min.x + text.rect.width() * 0.5,
                    text.rect.min.y + text.rect.height() * 0.5,
                ))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("production header should expose {label} selector text"))
}

fn header_switch_center(plan: &SurfacePaintPlan) -> Point {
    let a = header_text_center(plan, "A");
    let b = header_text_center(plan, "B");
    let center = plan
        .primitives
        .iter()
        .find_map(|primitive| match primitive {
            PaintPrimitive::Svg(svg) => {
                let center = Point::new(
                    svg.rect.min.x + svg.rect.width() * 0.5,
                    svg.rect.min.y + svg.rect.height() * 0.5,
                );
                (center.x > a.x && center.x < b.x).then_some(center)
            }
            _ => None,
        })
        .expect("center switch should expose a directional chevron");
    assert!(
        b.x - a.x >= PUMP_VISUAL_METRICS.icon_hit + 2.0 * PUMP_VISUAL_METRICS.space_4,
        "A/B selectors should leave room for the center switch hit target"
    );
    assert!(
        plan.primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::Svg(svg) if svg.rect.min.y < 70.0 && svg.rect.contains(center))
        }),
        "center switch should expose a directional chevron"
    );
    center
}

fn render_header_case(name: &str, state: HeaderCaptureState) -> (SurfacePaintPlan, Vec<u8>) {
    let store_path = std::env::temp_dir().join(format!(
        "pump-opt1124-header-{}-{}.bin",
        std::process::id(),
        name
    ));
    let (plan, pixels) = with_test_curve_slot_path(store_path.clone(), || {
        let params = Arc::new(PumpParams::new());
        if state == HeaderCaptureState::BActive {
            assert!(params.set_active_sound(SoundSide::B));
        }
        let mut editor = RadiantPumpEditor::new(
            params,
            Arc::new(GuiStatus::default()),
            Arc::new(PumpAutomationQueue::default()),
            None,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );
        let initial = editor.paint_plan().clone();
        let header_icons = header_svg_centers(&initial);
        assert!(
            header_icons.len() >= 3,
            "production header should expose undo/redo and the center switch chevron"
        );
        let switch_center = header_switch_center(&initial);
        let a_center = header_text_center(&initial, "A");
        assert!(
            a_center.x < switch_center.x,
            "A should remain left of the center switch"
        );
        match state {
            HeaderCaptureState::Normal
            | HeaderCaptureState::AActive
            | HeaderCaptureState::BActive => {}
            HeaderCaptureState::Hovered => {
                editor.dispatch_event(Event::pointer_move(switch_center));
            }
            HeaderCaptureState::CopyHovered => {
                editor.dispatch_event(Event::pointer_move(switch_center));
            }
            HeaderCaptureState::AHovered => {
                editor.dispatch_event(Event::pointer_move(header_text_center(&initial, "A")));
            }
            HeaderCaptureState::BHovered => {
                editor.dispatch_event(Event::pointer_move(header_text_center(&initial, "B")));
            }
            HeaderCaptureState::Pressed => {
                editor.dispatch_event(Event::pointer_move(switch_center));
                editor.dispatch_event(Event::primary_press(switch_center));
            }
            HeaderCaptureState::Disabled => {
                editor.dispatch_event(Event::pointer_move(header_icons[0]));
            }
        }
        let plan = editor.paint_plan().clone();
        let mut renderer = toybox::radiant_gui::bundled_offscreen_capture(
            Vector2::new(WINDOW_WIDTH as f32, WINDOW_HEIGHT as f32),
            DpiScale::ONE,
        )
        .expect("Vello offscreen adapter should be available for header screenshots");
        let pixels = renderer
            .capture(&plan)
            .expect("production header paint plan should render through Vello");
        (plan, pixels)
    });
    let _ = fs::remove_file(store_path);
    image::save_buffer_with_format(
        screenshot_root().join(format!("{name}.png")),
        &pixels,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        ColorType::Rgba8,
        ImageFormat::Png,
    )
    .expect("header screenshot PNG should be writable");
    (plan, pixels)
}

fn render_case_with_bypass(
    name: &str,
    width: u32,
    height: u32,
    dpi: DpiScale,
    bypassed: bool,
) -> (SurfacePaintPlan, Vec<u8>) {
    let store_path = std::env::temp_dir().join(format!(
        "pump-opt1122-screenshot-slots-{}-{}.bin",
        std::process::id(),
        name
    ));
    let (plan, pixels) = with_test_curve_slot_path(store_path.clone(), || {
        let params = Arc::new(PumpParams::new());
        params.set_bypass(f32::from(bypassed));
        for (index, seed) in quick_slot_seeds().iter().take(8).enumerate() {
            assert!(params.set_global_curve_slot_curve(index, &seed.curve));
        }
        let mut editor = RadiantPumpEditor::new(
            params,
            Arc::new(GuiStatus::default()),
            Arc::new(PumpAutomationQueue::default()),
            None,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );
        editor.resize(width, height);
        let plan = editor.paint_plan().clone();
        let mut renderer = toybox::radiant_gui::bundled_offscreen_capture(
            Vector2::new(width as f32, height as f32),
            dpi,
        )
        .expect("Vello offscreen adapter should be available for screenshot tests");
        let pixels = renderer
            .capture(&plan)
            .expect("Radiant paint plan should render through Vello");
        (plan, pixels)
    });
    let _ = fs::remove_file(store_path);
    let (physical_width, physical_height) = (
        (width as f32 * dpi.factor()).ceil() as u32,
        (height as f32 * dpi.factor()).ceil() as u32,
    );
    assert_eq!(
        pixels.len(),
        physical_width as usize * physical_height as usize * 4
    );
    image::save_buffer_with_format(
        screenshot_root().join(format!("{name}.png")),
        &pixels,
        physical_width,
        physical_height,
        ColorType::Rgba8,
        ImageFormat::Png,
    )
    .expect("screenshot PNG should be writable");
    (plan, pixels)
}

fn render_non_default_active_meter_case(
    name: &str,
    width: u32,
    height: u32,
    dpi: DpiScale,
) -> (SurfacePaintPlan, Vec<u8>) {
    let store_path = std::env::temp_dir().join(format!(
        "pump-opt1134-screenshot-fixture-{}-{}.bin",
        std::process::id(),
        name
    ));
    let (plan, pixels) = with_test_curve_slot_path(store_path.clone(), || {
        let params = Arc::new(PumpParams::new());
        params.set_depth_db(48.0);
        params.set_floor_db(-18.0);
        params.set_phase_offset(0.23);
        params.set_timing_mode(crate::params::TIMING_MODE_FREE as f32);
        params.set_free_rate_hz(2.5);
        params.set_smooth(0.62);
        params.set_mix(0.37);
        params.set_output_gain_db(-3.5);
        let status = Arc::new(GuiStatus::default());
        status.update(
            0.25,
            0.5,
            crate::GuiTransportTelemetry {
                is_playing: true,
                transport_is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.25,
                tempo_bpm: 120.0,
                beats_per_cycle: 4.0,
            },
        );
        let mut editor = RadiantPumpEditor::new(
            params,
            Arc::clone(&status),
            Arc::new(PumpAutomationQueue::default()),
            None,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );
        editor.resize(width, height);
        // Refresh immediately before the retained frame is painted so the
        // active meter cannot age out while the fixture is being assembled.
        status.publish_gain_reduction(0.5, true);
        let plan = editor.paint_plan().clone();
        let mut renderer = toybox::radiant_gui::bundled_offscreen_capture(
            Vector2::new(width as f32, height as f32),
            dpi,
        )
        .expect("Vello offscreen adapter should be available for screenshot tests");
        let pixels = renderer
            .capture(&plan)
            .expect("Radiant paint plan should render through Vello");
        (plan, pixels)
    });
    let _ = fs::remove_file(store_path);
    let (physical_width, physical_height) = (
        (width as f32 * dpi.factor()).ceil() as u32,
        (height as f32 * dpi.factor()).ceil() as u32,
    );
    assert_eq!(
        pixels.len(),
        physical_width as usize * physical_height as usize * 4
    );
    image::save_buffer_with_format(
        screenshot_root().join(format!("{name}.png")),
        &pixels,
        physical_width,
        physical_height,
        ColorType::Rgba8,
        ImageFormat::Png,
    )
    .expect("screenshot PNG should be writable");
    (plan, pixels)
}

const GALLERY_WIDTH: u32 = 720;
const GALLERY_HEIGHT: u32 = 360;
const GALLERY_LABEL_WIDTH: f32 = 104.0;
const GALLERY_CELL_WIDTH: f32 = 72.0;
const GALLERY_CELL_HEIGHT: f32 = 56.0;
const GALLERY_CELL_STEP_X: f32 = 76.0;
const GALLERY_ROW_STEP_Y: f32 = 65.0;
const GALLERY_TOP: f32 = 24.0;
const GALLERY_WIDGET_BASE: u64 = 9_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GalleryState {
    Default,
    Hover,
    Pressed,
    Selected,
    Disabled,
    Focused,
    Automation,
    SelectedFocusedAutomation,
}

impl GalleryState {
    const ALL: [Self; 8] = [
        Self::Default,
        Self::Hover,
        Self::Pressed,
        Self::Selected,
        Self::Disabled,
        Self::Focused,
        Self::Automation,
        Self::SelectedFocusedAutomation,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Hover => "Hover",
            Self::Pressed => "Pressed",
            Self::Selected => "Selected",
            Self::Disabled => "Disabled+sel+auto",
            Self::Focused => "Focused",
            Self::Automation => "Automation active",
            Self::SelectedFocusedAutomation => "Selected+focus+auto",
        }
    }

    fn is_disabled(self) -> bool {
        matches!(self, Self::Disabled)
    }

    fn is_focused(self) -> bool {
        matches!(self, Self::Focused | Self::SelectedFocusedAutomation)
    }

    fn is_automation(self) -> bool {
        matches!(
            self,
            Self::Disabled | Self::Automation | Self::SelectedFocusedAutomation
        )
    }

    fn is_selected(self) -> bool {
        matches!(
            self,
            Self::Selected | Self::Disabled | Self::SelectedFocusedAutomation
        )
    }
}

fn gallery_cell(row: usize, column: usize) -> Rect {
    Rect::from_xy_size(
        GALLERY_LABEL_WIDTH + column as f32 * GALLERY_CELL_STEP_X,
        GALLERY_TOP + row as f32 * GALLERY_ROW_STEP_Y,
        GALLERY_CELL_WIDTH,
        GALLERY_CELL_HEIGHT,
    )
}

fn gallery_widget_id(row: usize, column: usize) -> u64 {
    GALLERY_WIDGET_BASE + (row * GalleryState::ALL.len() + column) as u64
}

fn gallery_fill(theme: &radiant::theme::ThemeTokens, state: GalleryState) -> Rgba8 {
    // Disabled wins over every other visual state, then pressed/selected,
    // followed by hover and the neutral surface.
    if state.is_disabled() {
        theme.control_disabled_fill
    } else if matches!(state, GalleryState::Pressed) {
        theme.accent_copper.with_alpha(96)
    } else if state.is_selected() {
        theme.accent_mint.with_alpha(56)
    } else if matches!(state, GalleryState::Hover) {
        theme.surface_overlay
    } else {
        theme.surface_base
    }
}

fn push_gallery_text(
    plan: &mut SurfacePaintPlan,
    widget_id: u64,
    text: &'static str,
    rect: Rect,
    font_size: f32,
    color: Rgba8,
) {
    plan.primitives.push(PaintPrimitive::Text(PaintTextRun {
        widget_id,
        text: PaintText::from_static(text),
        rect,
        font_size,
        baseline: None,
        color,
        align: PaintTextAlign::Center,
        wrap: TextWrap::None,
    }));
}

fn gallery_widget_state(state: GalleryState) -> WidgetState {
    WidgetState {
        hovered: matches!(state, GalleryState::Hover),
        pressed: matches!(state, GalleryState::Pressed),
        selected: state.is_selected(),
        disabled: state.is_disabled(),
        focused: state.is_focused(),
        automation_active: state.is_automation(),
        ..WidgetState::default()
    }
}

fn paint_gallery_widget<W: Widget>(
    plan: &mut SurfacePaintPlan,
    widget: &mut W,
    rect: Rect,
    state: GalleryState,
    theme: &radiant::theme::ThemeTokens,
) {
    widget.common_mut().state = gallery_widget_state(state);
    widget.append_paint(&mut plan.primitives, rect, &LayoutOutput::default(), theme);
}

/// Build the deterministic component-state gallery from Radiant paint primitives.
fn component_state_gallery_plan() -> SurfacePaintPlan {
    let theme = pump_theme();
    let meter = pump_meter_colors();
    let mut plan = SurfacePaintPlan::empty(&theme);
    for (column, state) in GalleryState::ALL.into_iter().enumerate() {
        push_gallery_text(
            &mut plan,
            GALLERY_WIDGET_BASE,
            state.label(),
            Rect::from_xy_size(
                GALLERY_LABEL_WIDTH + column as f32 * GALLERY_CELL_STEP_X,
                4.0,
                GALLERY_CELL_WIDTH,
                16.0,
            ),
            6.0,
            theme.text_muted,
        );
    }
    for row in 0..5 {
        push_gallery_text(
            &mut plan,
            GALLERY_WIDGET_BASE + row as u64,
            [
                "Knob",
                "Dropdown",
                "Icon button",
                "Panel/divider",
                "Meter segments",
            ][row],
            Rect::from_xy_size(
                4.0,
                GALLERY_TOP + row as f32 * GALLERY_ROW_STEP_Y,
                96.0,
                16.0,
            ),
            PUMP_TYPOGRAPHY.control_label.0,
            theme.text_primary,
        );
        for (column, state) in GalleryState::ALL.into_iter().enumerate() {
            let rect = gallery_cell(row, column);
            let widget_id = gallery_widget_id(row, column);
            let sizing = WidgetSizing::fixed(Vector2::new(rect.width(), rect.height()));
            match row {
                0 => {
                    let mut widget = KnobWidget::new(widget_id, 0.5);
                    paint_gallery_widget(&mut plan, &mut widget, rect, state, &theme);
                }
                1 => {
                    let mut widget = ButtonWidget::new(widget_id, "1/4", sizing)
                        .with_trailing_icon_tint_cache(IconName::ChevronDown.tint_cache());
                    paint_gallery_widget(&mut plan, &mut widget, rect, state, &theme);
                }
                2 => {
                    let mut widget =
                        IconButtonWidget::new(widget_id, IconName::Settings.icon(), sizing);
                    paint_gallery_widget(&mut plan, &mut widget, rect, state, &theme);
                }
                3 => {
                    let mut widget = CardWidget::new(widget_id, sizing);
                    paint_gallery_widget(&mut plan, &mut widget, rect, state, &theme);
                    plan.primitives
                        .push(PaintPrimitive::StrokePolyline(PaintStrokePolyline {
                            widget_id,
                            points: std::sync::Arc::from([
                                Point::new(rect.min.x + 8.0, rect.min.y + 28.0),
                                Point::new(rect.max.x - 8.0, rect.min.y + 28.0),
                            ]),
                            color: theme.grid_strong,
                            width: PUMP_VISUAL_METRICS.divider,
                        }));
                }
                _ => {
                    let mut widget = CardWidget::new(widget_id, sizing);
                    paint_gallery_widget(&mut plan, &mut widget, rect, state, &theme);
                    for segment in 0..6 {
                        let y = rect.max.y
                            - 8.0
                            - segment as f32
                                * (PUMP_VISUAL_METRICS.meter_segment
                                    + PUMP_VISUAL_METRICS.meter_segment_gap);
                        plan.primitives
                            .push(PaintPrimitive::FillRect(PaintFillRect {
                                widget_id,
                                rect: Rect::from_xy_size(
                                    rect.min.x + 28.0,
                                    y,
                                    16.0,
                                    PUMP_VISUAL_METRICS.meter_segment,
                                ),
                                color: if state.is_disabled() {
                                    meter.track
                                } else if segment >= 4 {
                                    meter.hot
                                } else {
                                    meter.nominal
                                },
                            }));
                    }
                }
            }
        }
    }
    plan
}

fn gallery_vertical_state_marker_count(
    plan: &SurfacePaintPlan,
    widget_id: u64,
    bounds: Rect,
    leading: bool,
) -> usize {
    let expected_x = if leading {
        bounds.min.x + 2.0
    } else {
        bounds.max.x - 2.0
    };
    let expected_inset = (bounds.height() * 0.2).max(2.0);
    plan.primitives
        .iter()
        .filter(|primitive| {
            matches!(primitive, PaintPrimitive::StrokePolyline(marker)
                if marker.widget_id == widget_id
                    && marker.points.len() == 2
                    && (marker.width - 2.0).abs() < f32::EPSILON
                    && (marker.points[0].x - expected_x).abs() < f32::EPSILON
                    && (marker.points[1].x - expected_x).abs() < f32::EPSILON
                    && (marker.points[0].y - (bounds.min.y + expected_inset)).abs()
                        < f32::EPSILON
                    && (marker.points[1].y - (bounds.max.y - expected_inset)).abs()
                        < f32::EPSILON)
        })
        .count()
}

fn gallery_button_focus_ring_count(plan: &SurfacePaintPlan, widget_id: u64, bounds: Rect) -> usize {
    let focus_bounds = Rect::from_min_max(
        Point::new(bounds.min.x + 1.0, bounds.min.y + 1.0),
        Point::new(bounds.max.x - 1.0, bounds.max.y - 1.0),
    );
    let cut = (focus_bounds.height().min(focus_bounds.width()) * 0.18).clamp(4.0, 8.0);
    let expected_points: Arc<[Point]> = Arc::from([
        Point::new(focus_bounds.min.x, focus_bounds.min.y),
        Point::new(focus_bounds.max.x, focus_bounds.min.y),
        Point::new(focus_bounds.max.x, focus_bounds.max.y - cut),
        Point::new(focus_bounds.max.x - cut, focus_bounds.max.y),
        Point::new(focus_bounds.min.x, focus_bounds.max.y),
    ]);
    plan.primitives
        .iter()
        .filter(|primitive| {
            matches!(primitive, PaintPrimitive::StrokePolygon(stroke)
                if stroke.widget_id == widget_id
                    && stroke.points == expected_points
                    && (stroke.width - 2.0).abs() < f32::EPSILON)
        })
        .count()
}

fn gallery_button_fill_count(plan: &SurfacePaintPlan, widget_id: u64, color: Rgba8) -> usize {
    plan.primitives
        .iter()
        .filter(|primitive| {
            matches!(primitive, PaintPrimitive::FillPolygon(fill)
                if fill.widget_id == widget_id && fill.color == color)
        })
        .count()
}

fn assert_component_state_gallery_contract(plan: &SurfacePaintPlan) {
    let disabled_state = gallery_widget_state(GalleryState::Disabled);
    assert!(disabled_state.disabled);
    assert!(disabled_state.selected);
    assert!(disabled_state.automation_active);
    assert_eq!(
        plan.primitives
            .iter()
            .filter(|primitive| matches!(primitive, PaintPrimitive::Text(_)))
            .count(),
        21
    );
    assert_eq!(
        plan.primitives
            .iter()
            .filter(|primitive| matches!(primitive, PaintPrimitive::Svg(_)))
            .count(),
        16,
        "dropdown and icon-button icons must remain retained SVGs"
    );
    assert!(plan.primitives.iter().all(|primitive| match primitive {
        PaintPrimitive::FillRect(fill) =>
            fill.rect.max.x <= GALLERY_WIDTH as f32 && fill.rect.max.y <= GALLERY_HEIGHT as f32,
        PaintPrimitive::StrokeRect(stroke) =>
            stroke.rect.max.x <= GALLERY_WIDTH as f32 && stroke.rect.max.y <= GALLERY_HEIGHT as f32,
        PaintPrimitive::Text(text) =>
            text.rect.max.x <= GALLERY_WIDTH as f32
                && text.rect.max.y <= GALLERY_HEIGHT as f32
                && text.text.as_str().len() > 1,
        PaintPrimitive::Svg(svg) =>
            svg.rect.max.x <= GALLERY_WIDTH as f32 && svg.rect.max.y <= GALLERY_HEIGHT as f32,
        PaintPrimitive::StrokePolyline(polyline) => polyline
            .points
            .iter()
            .all(|point| point.x <= GALLERY_WIDTH as f32 && point.y <= GALLERY_HEIGHT as f32),
        _ => true,
    }));
    let selected_knob_id = gallery_widget_id(0, 3);
    let selected_knob_bounds = gallery_cell(0, 3);
    assert!(plan.primitives.iter().any(|primitive| {
        matches!(primitive, PaintPrimitive::StrokePolyline(line) if line.widget_id == selected_knob_id
            && line.points.len() == 2
            && (line.width - 2.0).abs() < f32::EPSILON
            && (line.points[0].x - line.points[1].x).abs() < f32::EPSILON
            && line.points[0].y < selected_knob_bounds.min.y + selected_knob_bounds.height() * 0.35)
    }));

    for row in [1, 2] {
        let selected_id = gallery_widget_id(row, 3);
        let selected_bounds = gallery_cell(row, 3);
        assert_eq!(
            gallery_vertical_state_marker_count(plan, selected_id, selected_bounds, true),
            1,
            "selected actual-widget cell must paint one 2px leading marker (row {row})"
        );
        assert_eq!(
            gallery_vertical_state_marker_count(plan, selected_id, selected_bounds, false),
            0,
            "selected actual-widget cell must not duplicate the marker at the trailing edge (row {row})"
        );

        let combined_id = gallery_widget_id(row, 7);
        let combined_bounds = gallery_cell(row, 7);
        assert_eq!(
            gallery_button_focus_ring_count(plan, combined_id, combined_bounds),
            1,
            "combined actual-widget cell must use the shared in-bounds focus geometry (row {row})"
        );
        assert_eq!(
            gallery_vertical_state_marker_count(plan, combined_id, combined_bounds, true),
            1,
            "combined actual-widget cell must retain one leading selected marker (row {row})"
        );
        assert_eq!(
            gallery_vertical_state_marker_count(plan, combined_id, combined_bounds, false),
            1,
            "combined actual-widget cell must retain one trailing automation marker (row {row})"
        );
    }

    let combined_knob_id = gallery_widget_id(0, 7);
    let combined_knob_bounds = gallery_cell(0, 7);
    assert!(plan.primitives.iter().any(|primitive| {
        matches!(primitive, PaintPrimitive::StrokePolyline(line) if line.widget_id == combined_knob_id
            && (line.width - 2.0).abs() < f32::EPSILON
            && line.points.len() == 2
            && (line.points[0].x - line.points[1].x).abs() < f32::EPSILON
            && (line.points[0].x - (combined_knob_bounds.max.x - 2.0)).abs() < f32::EPSILON)
    }));
    for row in 0..=2 {
        let disabled_id = gallery_widget_id(row, 4);
        let disabled_bounds = gallery_cell(row, 4);
        assert_eq!(
            gallery_vertical_state_marker_count(plan, disabled_id, disabled_bounds, true),
            0,
            "disabled actual-widget cell must suppress selected state markers (row {row})"
        );
        assert_eq!(
            gallery_vertical_state_marker_count(plan, disabled_id, disabled_bounds, false),
            0,
            "disabled actual-widget cell must suppress automation state markers (row {row})"
        );
    }
    for row in [1, 2] {
        let disabled_id = gallery_widget_id(row, 4);
        assert_eq!(
            gallery_button_fill_count(
                plan,
                disabled_id,
                pump_theme().control_disabled_fill
            ),
            1,
            "disabled selected+automation actual-button cell must paint exactly one disabled chrome fill (row {row})"
        );
    }
    assert_eq!(
        gallery_fill(&pump_theme(), GalleryState::Disabled),
        pump_theme().control_disabled_fill
    );
    assert_eq!(
        gallery_fill(&pump_theme(), GalleryState::Pressed),
        pump_theme().accent_copper.with_alpha(96)
    );
}

fn render_gallery_case(name: &str, dpi: DpiScale) -> Vec<u8> {
    let plan = component_state_gallery_plan();
    let mut renderer = toybox::radiant_gui::bundled_offscreen_capture(
        Vector2::new(GALLERY_WIDTH as f32, GALLERY_HEIGHT as f32),
        dpi,
    )
    .expect("Vello offscreen adapter should be available for component gallery");
    let pixels = renderer
        .capture(&plan)
        .expect("component gallery should render through Vello");
    let width = (GALLERY_WIDTH as f32 * dpi.factor()).ceil() as u32;
    let height = (GALLERY_HEIGHT as f32 * dpi.factor()).ceil() as u32;
    assert_eq!(pixels.len(), width as usize * height as usize * 4);
    image::save_buffer_with_format(
        screenshot_root().join(format!("{name}.png")),
        &pixels,
        width,
        height,
        ColorType::Rgba8,
        ImageFormat::Png,
    )
    .expect("component gallery PNG should be writable");
    pixels
}

fn assert_layout_contract(plan: &SurfacePaintPlan, width: u32, height: u32) {
    let viewport = Rect::from_xy_size(0.0, 0.0, width as f32, height as f32);
    let mut labels = Vec::new();
    let mut timing_label_widths = Vec::new();
    let sync_trigger_label = format!(
        "Sync {}",
        sync_division_label(crate::params::DEFAULT_SYNC_DIVISION_INDEX)
    );
    let mut curve_points = 0;
    let mut shaped_curve_slots = 0;
    let mut frame_paths = 0;
    for primitive in &plan.primitives {
        let rect = match primitive {
            PaintPrimitive::FillRect(fill) => Some(fill.rect),
            PaintPrimitive::StrokeRect(stroke) => Some(stroke.rect),
            PaintPrimitive::Text(text) => {
                labels.push(text.text.as_str());
                if text.text.as_str() == sync_trigger_label {
                    timing_label_widths.push(text.rect.width());
                }
                Some(text.rect)
            }
            PaintPrimitive::OverlayPanel(panel) => Some(panel.rect),
            PaintPrimitive::TextInput(input) => Some(input.rect),
            PaintPrimitive::Image(image) => Some(image.rect),
            PaintPrimitive::FillPath(path) => {
                if path.widget_id == 0 {
                    frame_paths += 1;
                }
                for command in path.path.commands() {
                    let points = match command {
                        PaintPathCommand::MoveTo(point) | PaintPathCommand::LineTo(point) => {
                            vec![*point]
                        }
                        PaintPathCommand::QuadTo { control, to } => vec![*control, *to],
                        PaintPathCommand::CurveTo {
                            control1,
                            control2,
                            to,
                        } => vec![*control1, *control2, *to],
                        PaintPathCommand::Close => Vec::new(),
                    };
                    assert!(
                        points.iter().all(|point| viewport.contains(*point)),
                        "rounded frame path escaped host bounds"
                    );
                }
                None
            }
            PaintPrimitive::StrokePolyline(polyline) => {
                curve_points = curve_points.max(polyline.points.len());
                let (min_y, max_y) = polyline
                    .points
                    .iter()
                    .map(|point| point.y)
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), y| {
                        (min.min(y), max.max(y))
                    });
                let (min_x, max_x) = polyline
                    .points
                    .iter()
                    .map(|point| point.x)
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), x| {
                        (min.min(x), max.max(x))
                    });
                if polyline.points.len() >= 8 && max_y - min_y > 8.0 && max_x - min_x > 40.0 {
                    shaped_curve_slots += 1;
                }
                None
            }
            _ => None,
        };
        if let Some(rect) = rect {
            assert!(
                viewport.contains(rect.min) && viewport.contains(rect.max),
                "layout primitive escaped host bounds: {rect:?} in {viewport:?}"
            );
        }
    }
    assert!(
        curve_points >= 8,
        "curve editor must remain the dominant flexible paint"
    );
    assert!(
        frame_paths >= 1,
        "editor should paint a rounded outer frame and card surfaces"
    );
    assert!(
        shaped_curve_slots >= 8,
        "all eight global curve slots must paint seeded curves instead of flat empty tiles"
    );
    for label in [
        "PUMP",
        "PORTALSURFER",
        "/",
        sync_trigger_label.as_str(),
        "SYNC",
        "SWING",
        "SMOOTH",
        "MIX",
        "OUTPUT",
    ] {
        assert!(labels.contains(&label), "missing editor label {label:?}");
    }
    for label in ["SWING", "SMOOTH", "MIX", "OUTPUT"] {
        assert!(
            plan.primitives.iter().any(|primitive| {
                matches!(
                    primitive,
                    PaintPrimitive::Text(text)
                        if text.text.as_str() == label
                            && text.align == PaintTextAlign::Center
                )
            }),
            "knob label {label:?} should be centered"
        );
    }
    assert!(
        timing_label_widths.iter().all(|width| *width >= 60.0),
        "timing labels need a full-width text cell"
    );
    for obsolete_label in [
        "Previous preset",
        "Next preset",
        "Favorite preset",
        "Add preset",
        "Save preset",
    ] {
        assert!(
            !labels.contains(&obsolete_label),
            "preset action label {obsolete_label:?} must not remain in the header"
        );
    }
    assert!(
        labels
            .iter()
            .all(|label| !label.starts_with("Grid ") && !label.contains("A active · A/B")),
        "bottom status text must not be painted"
    );
    assert!(
        labels
            .iter()
            .all(|label| !label.contains("traffic") && !label.contains("titlebar")),
        "host traffic-light/titlebar chrome must not be painted by the plugin"
    );
}

#[test]
fn pump_editor_screenshots_cover_supported_sizes_and_fractional_scale() {
    for (name, width, height) in [
        ("pump-min-640x400", MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT),
        ("pump-default-640x400", WINDOW_WIDTH, WINDOW_HEIGHT),
        ("pump-max-1280x800", MAX_WINDOW_WIDTH, MAX_WINDOW_HEIGHT),
    ] {
        let (plan, pixels) = render_case(name, width, height, DpiScale::ONE);
        assert!(!pixels.iter().all(|byte| *byte == plan.clear_color.r));
        assert_layout_contract(&plan, width, height);
    }

    let (fractional_plan, fractional_pixels) = render_case(
        "pump-default-640x400-dpi-1_25",
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        DpiScale::new(1.25),
    );
    assert_layout_contract(&fractional_plan, WINDOW_WIDTH, WINDOW_HEIGHT);
    assert!(!fractional_pixels.is_empty());
}

#[test]
fn pump_editor_screenshots_capture_explicit_active_and_bypassed_states() {
    let (active, _) = render_case_with_bypass(
        "pump-bypass-active-640x400",
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        DpiScale::ONE,
        false,
    );
    let (bypassed, _) = render_case_with_bypass(
        "pump-bypass-bypassed-640x400",
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        DpiScale::ONE,
        true,
    );
    assert!(active.primitives.iter().any(|primitive| {
        matches!(
            primitive,
            PaintPrimitive::Text(text) if text.text.as_str() == "ACTIVE"
        )
    }));
    assert!(bypassed.primitives.iter().any(|primitive| {
        matches!(
            primitive,
            PaintPrimitive::Text(text) if text.text.as_str() == "BYPASSED"
        )
    }));
}

#[test]
fn pump_editor_header_captures_production_interaction_states() {
    let (normal, normal_pixels) =
        render_header_case("pump-header-normal-640x400", HeaderCaptureState::Normal);
    let (hovered, hovered_pixels) =
        render_header_case("pump-header-hovered-640x400", HeaderCaptureState::Hovered);
    let (_, _) = render_header_case(
        "pump-header-copy-hovered-640x400",
        HeaderCaptureState::CopyHovered,
    );
    let (_, _) = render_header_case(
        "pump-header-a-hovered-640x400",
        HeaderCaptureState::AHovered,
    );
    let (_, _) = render_header_case(
        "pump-header-b-hovered-640x400",
        HeaderCaptureState::BHovered,
    );
    let (pressed, pressed_pixels) =
        render_header_case("pump-header-pressed-640x400", HeaderCaptureState::Pressed);
    let (disabled, _) =
        render_header_case("pump-header-disabled-640x400", HeaderCaptureState::Disabled);
    let (a_active, _) =
        render_header_case("pump-header-a-active-640x400", HeaderCaptureState::AActive);
    let (b_active, _) =
        render_header_case("pump-header-b-active-640x400", HeaderCaptureState::BActive);

    for plan in [&normal, &hovered, &pressed, &disabled, &a_active, &b_active] {
        assert!(plan.primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::Text(text) if text.text.as_str() == "PUMP")
        }));
    }
    assert_ne!(
        normal_pixels, hovered_pixels,
        "hovered header should repaint"
    );
    assert_ne!(
        normal_pixels, pressed_pixels,
        "pressed header should repaint"
    );
    assert!(disabled.primitives.iter().all(|primitive| {
        !matches!(primitive, PaintPrimitive::Text(text) if text.text.as_str() == "Settings are not available in this build")
    }));
    for plan in [&a_active, &b_active] {
        assert!(plan.primitives.iter().any(|primitive| {
            matches!(primitive, PaintPrimitive::Text(text) if matches!(text.text.as_str(), "A" | "B"))
        }));
    }
}

#[test]
fn pump_editor_screenshot_fixture_renders_non_default_deck_and_active_meter() {
    let (plan, pixels) = render_non_default_active_meter_case(
        "pump-non-default-active-meter-640x400",
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        DpiScale::ONE,
    );
    assert!(!pixels.is_empty());
    for expected in ["FREE", "Hz", "RATE", "2.50 Hz", "62%", "37%", "-3.5 dB"] {
        assert!(
            plan.primitives.iter().any(
                |primitive| matches!(primitive, PaintPrimitive::Text(text) if text.text.contains(expected))
            ),
            "non-default screenshot fixture should expose {expected}"
        );
    }
    let meter = pump_meter_colors();
    assert!(plan.primitives.iter().any(|primitive| {
        matches!(
            primitive,
            PaintPrimitive::FillRect(fill)
                if fill.color == meter.nominal || fill.color == meter.hot
        )
    }));
}

#[test]
fn pump_component_state_gallery_captures_both_pixel_scales() {
    let plan = component_state_gallery_plan();
    assert_component_state_gallery_contract(&plan);
    let one_x = render_gallery_case("pump-components-states-720x360-1x", DpiScale::ONE);
    let two_x = render_gallery_case("pump-components-states-720x360-2x", DpiScale::new(2.0));
    assert_eq!(one_x.len(), (GALLERY_WIDTH * GALLERY_HEIGHT * 4) as usize);
    assert_eq!(
        two_x.len(),
        (GALLERY_WIDTH * 2 * GALLERY_HEIGHT * 2 * 4) as usize
    );
}

#[test]
fn pump_editor_factory_fingerprint_is_stable() {
    let (first, _) = render_case(
        "pump-fingerprint-first",
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        DpiScale::ONE,
    );
    let (second, _) = render_case(
        "pump-fingerprint-second",
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        DpiScale::ONE,
    );
    assert_eq!(first.primitives, second.primitives);
}
