use std::sync::Arc;

use toybox::gui::declarative::{LayoutDiagnosticsMode, LayoutNodeDiagnostic, LayoutNodeKind};
use toybox::gui::{Point, Rect, Size, render_spec_to_frame, screenshot_harness};

use super::{
    resolve_vertical_slot_heights, AutomationQueue, GuiState, GuiStatus, PumpParams, CURVE_KEY,
    DIVISION_KEY, PRESET_DROPDOWN_KEY, WINDOW_HEIGHT, WINDOW_WIDTH,
};
use crate::params::SYNC_DIVISIONS;

struct CapturedFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    diagnostics: Vec<LayoutNodeDiagnostic>,
}

struct DropdownGeometry {
    preset_dropdown: Rect,
    division_dropdown: Rect,
    curve_region: Rect,
}

#[test]
fn screenshot_renders_initial_ui() {
    let params = Arc::new(PumpParams::new());
    let status = Arc::new(GuiStatus::default());
    let queue = Arc::new(AutomationQueue::default());
    let state = GuiState::new(params, status, queue, None);

    screenshot_harness::capture_initial_ui_screenshots_if_enabled(
        env!("CARGO_PKG_NAME"),
        Size {
            width: WINDOW_WIDTH,
            height: WINDOW_HEIGHT,
        },
        |input| state.build_ui(input),
    )
    .expect("failed to capture pump headless screenshots");
}

#[test]
fn dropdown_popups_render_over_curve_content_with_headless_click_open() {
    let params = Arc::new(PumpParams::new());
    for _ in 0..8 {
        if params.add_preset_from_current_state().is_none() {
            break;
        }
    }

    let status = Arc::new(GuiStatus::default());
    let queue = Arc::new(AutomationQueue::default());
    let preset_option_count = params.preset_bank_snapshot().presets.len().max(1);
    let division_option_count = SYNC_DIVISIONS.len();
    let state = GuiState::new(params, status, queue, None);

    let baseline = render_frame(&state, LayoutDiagnosticsMode::PerNode)
        .expect("baseline frame should render");
    let geometry = dropdown_geometry_from_diagnostics(
        &baseline.diagnostics,
        baseline.width,
        baseline.height,
    )
    .expect("dropdown geometry should resolve from diagnostics");

    let preset_open_menu = dropdown_menu_rect(
        geometry.preset_dropdown,
        preset_option_count,
        false,
        baseline.height,
    );
    assert!(
        rects_intersect(preset_open_menu, geometry.curve_region),
        "preset dropdown popup geometry should overlap curve region"
    );

    let division_open_menu = dropdown_menu_rect(
        geometry.division_dropdown,
        division_option_count,
        true,
        baseline.height,
    );
    assert!(
        rects_intersect(division_open_menu, geometry.curve_region),
        "division dropdown popup geometry should overlap curve region"
    );

    assert!(
        rect_contains_point(
            division_open_menu,
            dropdown_menu_row_point(geometry.division_dropdown, 1, true),
        ),
        "division popup geometry should include at least one selectable row over the curve"
    );
    assert!(
        rect_contains_point(
            preset_open_menu,
            dropdown_menu_row_point(geometry.preset_dropdown, 1, false),
        ),
        "preset popup geometry should include at least one selectable row"
    );

    let curve_variation = pixel_variation_count_in_rect(
        baseline.width,
        baseline.height,
        &baseline.pixels,
        geometry.curve_region,
    );
    assert!(
        curve_variation > 0,
        "curve region should contain rendered content under popup overlap area"
    );
}

fn render_frame(state: &GuiState, diagnostics_mode: LayoutDiagnosticsMode) -> Result<CapturedFrame, String> {
    let frame = render_spec_to_frame(
        Size {
            width: WINDOW_WIDTH,
            height: WINDOW_HEIGHT,
        },
        |input| {
            let mut spec = state.build_ui(input);
            spec.root.layout_diagnostics_mode = diagnostics_mode;
            spec
        },
    )?;

    Ok(CapturedFrame {
        width: frame.width,
        height: frame.height,
        pixels: frame.pixels,
        diagnostics: frame.render_result.node_layout_diagnostics,
    })
}

fn dropdown_geometry_from_diagnostics(
    diagnostics: &[LayoutNodeDiagnostic],
    width: u32,
    height: u32,
) -> Result<DropdownGeometry, String> {
    let preset_dropdown = find_dropdown_rect(diagnostics, PRESET_DROPDOWN_KEY)
        .ok_or_else(|| "missing preset-dropdown diagnostics rectangle".to_string())?;
    let division_dropdown = find_dropdown_rect(diagnostics, DIVISION_KEY)
        .ok_or_else(|| "missing division diagnostics rectangle".to_string())?;
    let curve_region = find_curve_rect(diagnostics).unwrap_or_else(|| {
        let (header_h, curve_h, _) = resolve_vertical_slot_heights(height);
        Rect {
            origin: Point {
                x: 0,
                y: header_h as i32,
            },
            size: Size {
                width,
                height: curve_h,
            },
        }
    });
    Ok(DropdownGeometry {
        preset_dropdown,
        division_dropdown,
        curve_region,
    })
}

fn find_dropdown_rect(entries: &[LayoutNodeDiagnostic], key: &str) -> Option<Rect> {
    let needle = format!("dropdown:{key}[");
    entries
        .iter()
        .find(|entry| entry.node_kind == LayoutNodeKind::Dropdown && entry.node_path.contains(&needle))
        .map(|entry| entry.resolved_rect)
}

fn find_curve_rect(entries: &[LayoutNodeDiagnostic]) -> Option<Rect> {
    let needle = format!("curve-editor:{CURVE_KEY}[");
    entries
        .iter()
        .find(|entry| {
            entry.node_kind == LayoutNodeKind::CurveEditor && entry.node_path.contains(&needle)
        })
        .map(|entry| entry.resolved_rect)
}

fn dropdown_menu_row_point(control_rect: Rect, row_index: usize, open_up: bool) -> Point {
    let row_height = control_rect.size.height as i32;
    let menu_row = row_index as i32 + 1;
    let y = if open_up {
        control_rect.origin.y - row_height * menu_row + (row_height / 2)
    } else {
        control_rect.origin.y + row_height * menu_row + (row_height / 2)
    };
    Point {
        x: control_rect.origin.x + (control_rect.size.width as i32 / 2),
        y,
    }
}

fn dropdown_menu_rect(
    control_rect: Rect,
    option_count: usize,
    open_up: bool,
    window_height: u32,
) -> Rect {
    let row_height = control_rect.size.height.max(1) as i32;
    let unclamped_height = row_height.saturating_mul(option_count.max(1) as i32);
    let menu_height = unclamped_height.min(window_height as i32);
    let origin_y = if open_up {
        (control_rect.origin.y - menu_height).max(0)
    } else {
        let max_origin = window_height as i32 - menu_height;
        (control_rect.origin.y + row_height).min(max_origin.max(0))
    };
    Rect {
        origin: Point {
            x: control_rect.origin.x,
            y: origin_y,
        },
        size: Size {
            width: control_rect.size.width,
            height: menu_height.max(0) as u32,
        },
    }
}

fn pixel_variation_count_in_rect(width: u32, height: u32, pixels: &[u8], rect: Rect) -> usize {
    if pixels.len() != (width as usize * height as usize * 4) {
        return 0;
    }
    let Some((x0, y0, x1, y1)) = clamp_rect(rect, width, height) else {
        return 0;
    };
    let stride = width as usize * 4;
    let mut first: Option<[u8; 4]> = None;
    let mut variation = 0usize;
    for y in y0..y1 {
        for x in x0..x1 {
            let offset = y as usize * stride + x as usize * 4;
            let rgba = [
                pixels[offset],
                pixels[offset + 1],
                pixels[offset + 2],
                pixels[offset + 3],
            ];
            if let Some(base) = first {
                if base != rgba {
                    variation += 1;
                }
            } else {
                first = Some(rgba);
            }
        }
    }
    variation
}

fn rects_intersect(a: Rect, b: Rect) -> bool {
    let ax0 = a.origin.x;
    let ay0 = a.origin.y;
    let ax1 = a.origin.x + a.size.width as i32;
    let ay1 = a.origin.y + a.size.height as i32;
    let bx0 = b.origin.x;
    let by0 = b.origin.y;
    let bx1 = b.origin.x + b.size.width as i32;
    let by1 = b.origin.y + b.size.height as i32;
    ax0 < bx1 && ax1 > bx0 && ay0 < by1 && ay1 > by0
}

fn rect_contains_point(rect: Rect, point: Point) -> bool {
    point.x >= rect.origin.x
        && point.y >= rect.origin.y
        && point.x < rect.origin.x + rect.size.width as i32
        && point.y < rect.origin.y + rect.size.height as i32
}

fn clamp_rect(rect: Rect, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let x0 = rect.origin.x.max(0) as u32;
    let y0 = rect.origin.y.max(0) as u32;
    let x1 = (rect.origin.x + rect.size.width as i32).clamp(0, width as i32) as u32;
    let y1 = (rect.origin.y + rect.size.height as i32).clamp(0, height as i32) as u32;
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    Some((x0, y0, x1, y1))
}
