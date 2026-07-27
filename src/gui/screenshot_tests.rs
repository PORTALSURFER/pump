//! Renderer-parity screenshots for the macOS Radiant editor.

use std::{fs, path::PathBuf, sync::Arc};

use image::{ColorType, ImageFormat};
use radiant::{
    gui::types::{Rect, Vector2},
    runtime::{PaintPathCommand, PaintPrimitive, SurfacePaintPlan},
    theme::DpiScale,
};
use toybox::clap::automation::AutomationQueue;

use super::{
    RadiantPumpEditor, MAX_WINDOW_HEIGHT, MAX_WINDOW_WIDTH, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
    WINDOW_HEIGHT, WINDOW_WIDTH,
};
use crate::{
    curve_presets::quick_slot_seeds,
    params::{with_test_curve_slot_path, PumpParams},
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
    let store_path = std::env::temp_dir().join(format!(
        "pump-opt1122-screenshot-slots-{}-{}.bin",
        std::process::id(),
        name
    ));
    let (plan, pixels) = with_test_curve_slot_path(store_path.clone(), || {
        let params = Arc::new(PumpParams::new());
        for (index, seed) in quick_slot_seeds().iter().take(6).enumerate() {
            assert!(params.set_global_curve_slot_curve(index, &seed.curve));
        }
        let mut editor = RadiantPumpEditor::new(
            params,
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
        );
        editor.resize(width, height);
        let plan = editor.paint_plan().clone();
        let mut renderer = radiant::gui_runtime::OffscreenVelloCapture::new(
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

fn assert_layout_contract(plan: &SurfacePaintPlan, width: u32, height: u32) {
    let viewport = Rect::from_xy_size(0.0, 0.0, width as f32, height as f32);
    let mut labels = Vec::new();
    let mut timing_label_widths = Vec::new();
    let mut curve_points = 0;
    let mut shaped_carousel_tiles = 0;
    let mut frame_paths = 0;
    for primitive in &plan.primitives {
        let rect = match primitive {
            PaintPrimitive::FillRect(fill) => Some(fill.rect),
            PaintPrimitive::StrokeRect(stroke) => Some(stroke.rect),
            PaintPrimitive::Text(text) => {
                labels.push(text.text.as_str());
                if matches!(text.text.as_str(), "Sync" | "Trigger" | "Mode") {
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
                    shaped_carousel_tiles += 1;
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
    assert_eq!(
        frame_paths, 1,
        "editor should paint one rounded outer frame"
    );
    assert!(
        shaped_carousel_tiles >= 6,
        "quick-shape carousel must paint seeded curves instead of flat empty tiles"
    );
    for label in [
        "PUMP", "Sync", "Trigger", "Mode", "Mix", "Depth", "Floor", "Phase", "Output", "Smooth",
    ] {
        assert!(labels.contains(&label), "missing editor label {label:?}");
    }
    assert!(
        timing_label_widths.iter().all(|width| *width >= 80.0),
        "timing labels need a full-width text cell"
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
        ("pump-min-720x540", MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT),
        ("pump-default-912x684", WINDOW_WIDTH, WINDOW_HEIGHT),
        ("pump-max-1440x1080", MAX_WINDOW_WIDTH, MAX_WINDOW_HEIGHT),
    ] {
        let (plan, pixels) = render_case(name, width, height, DpiScale::ONE);
        assert!(!pixels.iter().all(|byte| *byte == plan.clear_color.r));
        assert_layout_contract(&plan, width, height);
    }

    let (fractional_plan, fractional_pixels) = render_case(
        "pump-default-912x684-dpi-1_25",
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        DpiScale::new(1.25),
    );
    assert_layout_contract(&fractional_plan, WINDOW_WIDTH, WINDOW_HEIGHT);
    assert!(!fractional_pixels.is_empty());
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
