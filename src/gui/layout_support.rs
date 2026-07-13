use super::*;
pub(super) const fn u32_max(left: u32, right: u32) -> u32 {
    if left > right {
        left
    } else {
        right
    }
}

/// Enforce Pump's host-negotiated minimum window size.
pub(super) fn constrained_host_size(size: GuiSize) -> GuiSize {
    GuiSize {
        width: size.width.max(WINDOW_WIDTH),
        height: size.height.max(WINDOW_HEIGHT),
    }
}

pub(super) const fn resolve_vertical_slot_heights(total_height: u32) -> (u32, u32, u32, u32) {
    let clamped_total = u32_max(total_height, 1);
    let header_h =
        clamped_total.saturating_mul(HEADER_SECTION_WEIGHT as u32) / ROOT_SECTION_WEIGHT_SUM;
    let quick_shapes_h =
        clamped_total.saturating_mul(QUICK_SHAPES_SECTION_WEIGHT as u32) / ROOT_SECTION_WEIGHT_SUM;
    let controls_h =
        clamped_total.saturating_mul(CONTROLS_SECTION_WEIGHT as u32) / ROOT_SECTION_WEIGHT_SUM;
    let consumed = header_h
        .saturating_add(quick_shapes_h)
        .saturating_add(controls_h);
    let curve_h = clamped_total.saturating_sub(consumed);
    (header_h, curve_h, quick_shapes_h, controls_h)
}

/// Resolve curve editor height from the full curve slot height.
pub(super) const fn resolve_curve_editor_height(curve_slot_h: u32) -> u32 {
    curve_slot_h.saturating_sub(CURVE_VERTICAL_MARGIN.saturating_mul(2))
}

pub(super) fn resolve_runtime_controls_slot_widths(total_width: u32) -> (u32, u32) {
    let widths = weighted_slot_lengths(
        total_width.max(1),
        &[KNOBS_SECTION_WEIGHT, DROPDOWN_SECTION_WEIGHT],
    );
    (
        widths.first().copied().unwrap_or(1),
        widths.get(1).copied().unwrap_or(1),
    )
}

pub(super) fn scaled_line_height(text_scale: u32) -> u32 {
    BASE_CONTROL_LINE_UNIT.saturating_mul(text_scale.max(1))
}

/// Resolve a stable parameter id for one knob action key.
pub(super) fn knob_param_id(key: &str) -> Option<ClapId> {
    match key {
        MIX_KEY => Some(PARAM_MIX_ID),
        PHASE_KEY => Some(PARAM_PHASE_OFFSET_ID),
        OUTPUT_KEY => Some(PARAM_OUTPUT_GAIN_ID),
        _ => None,
    }
}

/// Shared Pump color/style tokens derived from the canonical Patchbay theme.
#[derive(Clone, Copy, Debug)]
pub(super) struct PumpTheme {
    pub(super) tokens: ThemeTokens,
    pub(super) preset_dirty_highlight: Color,
    pub(super) curve_bg: Color,
    pub(super) curve_border: Color,
    pub(super) curve_grid_vertical: Color,
    pub(super) curve_grid_emphasis: Color,
    pub(super) curve_grid_horizontal: Color,
    pub(super) curve_reference_line: Color,
    pub(super) curve_reference_label: Color,
    pub(super) curve_fill: Color,
    pub(super) curve_line: Color,
    pub(super) curve_line_highlight: Color,
    pub(super) curve_line_highlight_glow: Color,
    pub(super) preview_fill: Color,
    pub(super) preview_stroke: Color,
    pub(super) node_fill: Color,
    pub(super) node_hover_fill: Color,
    pub(super) node_selected_fill: Color,
    pub(super) node_stroke: Color,
    pub(super) node_hover_stroke: Color,
    pub(super) node_selected_stroke: Color,
    pub(super) node_hover_ring: Color,
    pub(super) node_selected_ring: Color,
    pub(super) playhead_dot_core: Color,
    pub(super) playhead_dot_glow: Color,
    pub(super) playhead_dot_stroke: Color,
    pub(super) meter_outline: Color,
    pub(super) meter_fill: Color,
    pub(super) version_label: Color,
    pub(super) snap_checkbox_bg: Color,
    pub(super) snap_checkbox_hover_bg: Color,
    pub(super) snap_checkbox_active_bg: Color,
    pub(super) snap_checkbox_outline: Color,
    pub(super) snap_checkbox_outline_hover: Color,
    pub(super) quick_slot_bg: Color,
    pub(super) quick_slot_hover_bg: Color,
    pub(super) quick_slot_store_hover_bg: Color,
    pub(super) quick_slot_active_bg: Color,
    pub(super) quick_slot_deviation_bg: Color,
    pub(super) quick_slot_outline: Color,
    pub(super) quick_slot_outline_hover: Color,
    pub(super) quick_slot_outline_store_hover: Color,
    pub(super) quick_slot_outline_deviation: Color,
    pub(super) quick_slot_curve: Color,
    pub(super) quick_slot_empty_curve: Color,
    pub(super) quick_slot_deviation_curve: Color,
}

impl PumpTheme {
    /// Return the canonical Pump GUI theme.
    pub(super) fn main(metrics: UiLayoutMetrics) -> Self {
        let palette = MainPalette::main();
        let mut tokens = ThemeTokens::main();
        tokens.typography.text_scale = metrics.text_scale;
        tokens.controls.knob_diameter = metrics.knob_diameter;
        tokens.controls.dropdown_height = metrics.dropdown_control_h;
        tokens.controls.button_height = metrics.button_control_h;
        Self {
            tokens,
            preset_dirty_highlight: palette.literals,
            curve_bg: palette.background_primary,
            curve_border: palette.ui_secondary,
            curve_grid_vertical: palette.background_secondary,
            curve_grid_emphasis: palette.ui_secondary,
            curve_grid_horizontal: palette.ui_secondary,
            curve_reference_line: Color::rgba(
                palette.text_muted.r,
                palette.text_muted.g,
                palette.text_muted.b,
                88,
            ),
            curve_reference_label: palette.text_muted,
            curve_fill: Color::rgba(
                palette.syntax_emphasis.r,
                palette.syntax_emphasis.g,
                palette.syntax_emphasis.b,
                64,
            ),
            curve_line: palette.syntax_emphasis,
            curve_line_highlight: palette.accent_focus,
            curve_line_highlight_glow: palette.text_primary,
            preview_fill: palette.literals,
            preview_stroke: palette.identifiers,
            node_fill: palette.text_primary,
            node_hover_fill: palette.identifiers,
            node_selected_fill: palette.accent_focus,
            node_stroke: palette.ui_secondary,
            node_hover_stroke: palette.syntax_emphasis,
            node_selected_stroke: palette.text_primary,
            node_hover_ring: palette.syntax_emphasis,
            node_selected_ring: palette.accent_focus,
            // Reserve magenta for transport position feedback. Editable curve
            // nodes and insertion previews use the shared palette across their
            // normal, hover, and selected states, so the playhead deliberately
            // sits outside that interaction vocabulary.
            playhead_dot_core: Color::rgb(255, 96, 208),
            playhead_dot_glow: Color::rgba(255, 96, 208, 112),
            playhead_dot_stroke: Color::rgb(255, 196, 232),
            meter_outline: palette.ui_secondary,
            meter_fill: palette.literals,
            version_label: palette.text_muted,
            snap_checkbox_bg: palette.background_primary,
            snap_checkbox_hover_bg: palette.background_secondary,
            snap_checkbox_active_bg: palette.literals,
            snap_checkbox_outline: palette.ui_secondary,
            snap_checkbox_outline_hover: palette.accent_focus,
            quick_slot_bg: palette.background_primary,
            quick_slot_hover_bg: palette.background_secondary,
            quick_slot_store_hover_bg: Color::rgba(
                palette.literals.r,
                palette.literals.g,
                palette.literals.b,
                48,
            ),
            quick_slot_active_bg: palette.ui_secondary,
            quick_slot_deviation_bg: Color::rgba(150, 30, 38, 96),
            quick_slot_outline: palette.ui_secondary,
            quick_slot_outline_hover: palette.accent_focus,
            quick_slot_outline_store_hover: palette.literals,
            quick_slot_outline_deviation: Color::rgba(255, 74, 88, 255),
            quick_slot_curve: palette.identifiers,
            quick_slot_empty_curve: palette.text_muted,
            quick_slot_deviation_curve: Color::rgba(255, 110, 116, 255),
        }
    }
}

/// Layout dimensions used to author Pump controls in design space.
///
/// Pump authors all widget geometry at a fixed logical design resolution.
/// Patchbay applies uniform root scaling at render time so host window size
/// changes do not alter declarative layout structure.
#[derive(Clone, Copy, Debug)]
pub(super) struct UiLayoutMetrics {
    pub(super) content_w: u32,
    pub(super) content_h: u32,
    pub(super) curve_h: u32,
    pub(super) curve_reference_gutter_width: u32,
    pub(super) meter_panel_width: u32,
    pub(super) dropdown_control_w: u32,
    pub(super) dropdown_control_h: u32,
    pub(super) button_control_h: u32,
    pub(super) quick_shape_button_w: u32,
    pub(super) quick_shape_button_h: u32,
    pub(super) transport_indicator_size: u32,
    pub(super) curve_size: Size,
    pub(super) knob_track_w: u32,
    pub(super) knob_diameter: u32,
    pub(super) text_scale: u32,
    pub(super) label_line_h: u32,
}

impl UiLayoutMetrics {
    /// Resolve all layout dimensions from the fixed design resolution.
    pub(super) fn design_space() -> Self {
        let content_w = WINDOW_WIDTH;
        let content_h = WINDOW_HEIGHT;
        let (_header_h, curve_h, quick_shapes_h, controls_h) =
            resolve_vertical_slot_heights(content_h);
        let (knobs_slot_w, dropdown_slot_w) = resolve_runtime_controls_slot_widths(content_w);
        let text_scale = BASE_TEXT_SCALE.max(1);
        let knob_track_width = knobs_slot_w.saturating_div(KNOBS_PER_ROW as u32);
        let knob_diameter = BASE_KNOB_DIAMETER.min(knob_track_width.max(1));
        let knob_track_w = knob_diameter.max(1);
        let label_line_h = scaled_line_height(text_scale);
        let expanded_control_h = controls_h
            .saturating_sub(label_line_h)
            .saturating_div(2)
            .max(BASE_DROPDOWN_CONTROL_H.max(1));
        let dropdown_control_h = expanded_control_h;
        let button_control_h = expanded_control_h;
        let dropdown_control_w = dropdown_slot_w.max(1);
        let quick_shape_button_w = content_w
            .saturating_div(QUICK_SHAPE_BUTTONS_PER_ROW as u32)
            .max(1);
        let quick_shape_button_h = quick_shapes_h.max(1);
        let transport_indicator_size = TRANSPORT_INDICATOR_SIZE.max(1);
        let curve_editor_h = resolve_curve_editor_height(curve_h).max(1);
        let curve_meter_widths = weighted_slot_lengths(
            content_w,
            &[CURVE_EDITOR_SECTION_WEIGHT, METER_SECTION_WEIGHT],
        );
        let curve_panel_width = curve_meter_widths[0];
        let curve_reference_gutter_width =
            CURVE_REFERENCE_GUTTER_WIDTH.min(curve_panel_width.saturating_sub(1));
        let curve_size = Size {
            width: curve_panel_width.saturating_sub(curve_reference_gutter_width),
            height: curve_editor_h,
        };
        let meter_panel_width = curve_meter_widths[1];
        Self {
            content_w,
            content_h,
            curve_h,
            curve_reference_gutter_width,
            meter_panel_width,
            dropdown_control_w,
            dropdown_control_h,
            button_control_h,
            quick_shape_button_w,
            quick_shape_button_h,
            transport_indicator_size,
            curve_size,
            knob_track_w,
            knob_diameter,
            text_scale,
            label_line_h,
        }
    }
}
