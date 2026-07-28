//! Pump's local visual-system contract.
//!
//! The reusable Radiant token fields remain the source of truth for colors;
//! these aliases and dimensions make Pump's composition explicit without
//! adding application-specific fields to Radiant.

use radiant::{gui::types::Rgba8, theme::ThemeTokens};

/// Return Pump's fixed dark-coral theme for every supported viewport tier.
pub(crate) fn pump_theme() -> ThemeTokens {
    let mut theme = ThemeTokens::dark();
    // Keep the canonical Radiant dark palette values fixed across Pump's
    // viewport sizes. Semantic names are documented in docs/visual-system.md.
    theme.clear_color = Rgba8::new(27, 30, 30, 255);
    theme.bg_primary = Rgba8::new(27, 30, 30, 255);
    theme.bg_secondary = Rgba8::new(27, 30, 30, 255);
    theme.bg_tertiary = Rgba8::new(27, 30, 30, 255);
    theme.surface_base = Rgba8::new(27, 30, 30, 255);
    theme.surface_raised = Rgba8::new(27, 30, 30, 255);
    theme.surface_overlay = Rgba8::new(42, 45, 45, 255);
    theme.border = Rgba8::new(58, 61, 61, 255);
    theme.border_emphasis = Rgba8::new(64, 67, 66, 255);
    theme.grid_strong = Rgba8::new(54, 57, 57, 255);
    theme.grid_soft = Rgba8::new(40, 43, 43, 255);
    theme.accent_mint = Rgba8::new(233, 88, 67, 255);
    theme.accent_copper = Rgba8::new(241, 108, 86, 255);
    theme.accent_danger = Rgba8::new(239, 76, 61, 255);
    theme.accent_warning = Rgba8::new(217, 151, 95, 255);
    theme.text_primary = Rgba8::new(216, 215, 211, 255);
    theme.text_muted = Rgba8::new(153, 155, 154, 255);
    theme.control_disabled_fill = Rgba8::new(36, 40, 41, 255);
    theme
}

/// Named geometry used by Pump's shared controls and editor composition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PumpVisualMetrics {
    /// Base spacing unit.
    pub(crate) base: f32,
    /// Four-pixel spacing.
    pub(crate) space_4: f32,
    /// Eight-pixel spacing.
    pub(crate) space_8: f32,
    /// Twelve-pixel spacing.
    pub(crate) space_12: f32,
    /// Sixteen-pixel spacing.
    pub(crate) space_16: f32,
    /// Standard surface padding.
    pub(crate) padding: f32,
    /// Standard control gap.
    pub(crate) gap: f32,
    /// Rounded panel radius.
    pub(crate) radius: f32,
    /// Border width.
    pub(crate) border: f32,
    /// Divider width.
    pub(crate) divider: f32,
    /// Control and dropdown height.
    pub(crate) control_height: f32,
    /// Minimum dropdown width.
    pub(crate) dropdown_min_width: f32,
    /// Minimum icon-button hit target.
    pub(crate) icon_hit: f32,
    /// Retained icon size.
    pub(crate) icon: f32,
    /// Standard knob diameter.
    pub(crate) knob: f32,
    /// Width reserved for one knob plus its label/value stack.
    pub(crate) knob_column: f32,
    /// Label line height.
    pub(crate) label_line: f32,
    /// Gain-reduction meter panel width.
    pub(crate) meter_panel: f32,
    /// Gain-reduction meter track width.
    pub(crate) meter_track: f32,
    /// Meter segment height.
    pub(crate) meter_segment: f32,
    /// Gap between meter segments.
    pub(crate) meter_segment_gap: f32,
    /// Existing Pump parameter-deck height.
    pub(crate) deck_height: f32,
}

/// Pump's exact visual dimensions.
pub(crate) const PUMP_VISUAL_METRICS: PumpVisualMetrics = PumpVisualMetrics {
    base: 4.0,
    space_4: 4.0,
    space_8: 8.0,
    space_12: 12.0,
    space_16: 16.0,
    padding: 12.0,
    gap: 8.0,
    radius: 8.0,
    border: 1.0,
    divider: 1.0,
    control_height: 32.0,
    dropdown_min_width: 96.0,
    icon_hit: 28.0,
    icon: 16.0,
    knob: 56.0,
    knob_column: 88.0,
    label_line: 16.0,
    meter_panel: 48.0,
    meter_track: 32.0,
    meter_segment: 4.0,
    meter_segment_gap: 2.0,
    deck_height: 96.0,
};

/// Typography roles for the target's license-safe text hierarchy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PumpTypography {
    /// Brand size and line height.
    pub(crate) brand: (f32, f32),
    /// Body size and line height.
    pub(crate) body: (f32, f32),
    /// Value size and line height.
    pub(crate) value: (f32, f32),
    /// Control-label size and line height.
    pub(crate) control_label: (f32, f32),
    /// Metadata size and line height.
    pub(crate) meta: (f32, f32),
}

/// Pump's target typography roles.
pub(crate) const PUMP_TYPOGRAPHY: PumpTypography = PumpTypography {
    brand: (22.0, 28.0),
    body: (14.0, 18.0),
    value: (12.0, 16.0),
    control_label: (10.0, 16.0),
    meta: (9.0, 14.0),
};

/// Meter-specific semantic colors derived from Pump's theme tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PumpMeterColors {
    /// Recessed meter track.
    pub(crate) track: Rgba8,
    /// Nominal active segment.
    pub(crate) nominal: Rgba8,
    /// Hot active segment.
    pub(crate) hot: Rgba8,
    /// Meter boundary and segment divider.
    pub(crate) border: Rgba8,
    /// Meter labels and values.
    pub(crate) text: Rgba8,
}

/// Resolve the Pump meter palette from the canonical theme.
pub(crate) fn pump_meter_colors() -> PumpMeterColors {
    let theme = pump_theme();
    PumpMeterColors {
        track: theme.grid_soft,
        nominal: theme.accent_copper,
        hot: theme.accent_danger,
        border: theme.border,
        text: theme.text_muted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pump_theme_is_fixed_and_uses_canonical_dark_coral_values() {
        let theme = pump_theme();
        assert_eq!(theme, pump_theme());
        assert_eq!(theme.clear_color, Rgba8::new(27, 30, 30, 255));
        assert_eq!(theme.accent_mint, Rgba8::new(233, 88, 67, 255));
        assert_eq!(theme.accent_copper, Rgba8::new(241, 108, 86, 255));
        assert_eq!(theme.text_primary, Rgba8::new(216, 215, 211, 255));
    }

    #[test]
    fn metrics_and_typography_match_the_visual_contract() {
        assert_eq!(PUMP_VISUAL_METRICS.base, 4.0);
        assert_eq!(PUMP_VISUAL_METRICS.control_height, 32.0);
        assert_eq!(PUMP_VISUAL_METRICS.knob, 56.0);
        assert_eq!(PUMP_VISUAL_METRICS.deck_height, 96.0);
        assert_eq!(PUMP_TYPOGRAPHY.brand, (22.0, 28.0));
        assert_eq!(PUMP_TYPOGRAPHY.meta, (9.0, 14.0));
    }

    #[test]
    fn meter_aliases_are_distinct_and_semantic() {
        let meter = pump_meter_colors();
        assert_eq!(meter.track, pump_theme().grid_soft);
        assert_eq!(meter.nominal, pump_theme().accent_copper);
        assert_eq!(meter.hot, pump_theme().accent_danger);
        assert_eq!(meter.border, pump_theme().border);
        assert_ne!(meter.nominal, meter.hot);
    }
}
