//! Pump's backend-neutral visual-system contract.
//!
//! The editor is rendered by GPUI, but the palette, spacing, typography, and
//! meter aliases remain a Pump-local contract so the native and screenshot
//! renderers use one set of values.

/// One RGBA color in the fixed Pump palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PumpColor {
    /// Red channel.
    pub(crate) r: u8,
    /// Green channel.
    pub(crate) g: u8,
    /// Blue channel.
    pub(crate) b: u8,
    /// Alpha channel.
    pub(crate) a: u8,
}

impl PumpColor {
    /// Construct an opaque RGB color.
    pub(crate) const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Return this color with a new alpha channel.
    pub(crate) const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// Pack this color as `0xRRGGBBAA` for GPUI's RGBA helper.
    pub(crate) const fn packed(self) -> u32 {
        ((self.r as u32) << 24) | ((self.g as u32) << 16) | ((self.b as u32) << 8) | self.a as u32
    }
}

/// Pump's fixed dark-coral palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PumpTheme {
    /// Canvas and primary surface.
    pub(crate) clear: PumpColor,
    /// Recessed display bed.
    pub(crate) display: PumpColor,
    /// Raised/overlay surface.
    pub(crate) surface_overlay: PumpColor,
    /// Standard border.
    pub(crate) border: PumpColor,
    /// Emphasized border.
    pub(crate) border_emphasis: PumpColor,
    /// Strong grid line.
    pub(crate) grid_strong: PumpColor,
    /// Soft grid line and meter track.
    pub(crate) grid_soft: PumpColor,
    /// Primary coral accent.
    pub(crate) accent_mint: PumpColor,
    /// Secondary coral accent.
    pub(crate) accent_copper: PumpColor,
    /// Warning color.
    pub(crate) accent_warning: PumpColor,
    /// Danger color.
    pub(crate) accent_danger: PumpColor,
    /// Primary text.
    pub(crate) text_primary: PumpColor,
    /// Muted text.
    pub(crate) text_muted: PumpColor,
    /// Disabled control fill.
    pub(crate) control_disabled_fill: PumpColor,
}

/// Return Pump's fixed dark-coral theme for every supported viewport tier.
pub(crate) const fn pump_theme() -> PumpTheme {
    PumpTheme {
        clear: PumpColor::rgb(39, 43, 40),
        display: PumpColor::rgb(31, 36, 34),
        surface_overlay: PumpColor::rgb(48, 55, 50),
        border: PumpColor::rgb(73, 83, 76),
        border_emphasis: PumpColor::rgb(91, 103, 94),
        grid_strong: PumpColor::rgb(48, 55, 50),
        grid_soft: PumpColor::rgb(37, 44, 40),
        accent_mint: PumpColor::rgb(233, 107, 80),
        accent_copper: PumpColor::rgb(233, 107, 80),
        accent_warning: PumpColor::rgb(215, 92, 73),
        accent_danger: PumpColor::rgb(215, 92, 73),
        text_primary: PumpColor::rgb(213, 216, 214),
        text_muted: PumpColor::rgb(162, 171, 164),
        control_disabled_fill: PumpColor::rgb(31, 36, 34),
    }
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
    padding: 16.0,
    gap: 8.0,
    radius: 1.0,
    border: 1.0,
    divider: 1.0,
    control_height: 24.0,
    dropdown_min_width: 80.0,
    icon_hit: 28.0,
    icon: 13.6,
    knob: 32.0,
    knob_column: 64.0,
    label_line: 12.0,
    meter_panel: 32.0,
    meter_track: 12.0,
    meter_segment: 3.4,
    meter_segment_gap: 1.7,
    deck_height: 80.0,
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
    brand: (17.0, 24.0),
    body: (11.0, 14.0),
    value: (11.0, 14.0),
    control_label: (9.0, 12.0),
    meta: (8.0, 10.0),
};

/// Meter-specific semantic colors derived from Pump's theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PumpMeterColors {
    /// Recessed meter track.
    pub(crate) track: PumpColor,
    /// Nominal active segment.
    pub(crate) nominal: PumpColor,
    /// Hot active segment.
    pub(crate) hot: PumpColor,
    /// Meter boundary and segment divider.
    pub(crate) border: PumpColor,
    /// Meter labels and values.
    pub(crate) text: PumpColor,
}

/// Resolve the Pump meter palette from the canonical theme.
pub(crate) const fn pump_meter_colors() -> PumpMeterColors {
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
    fn pump_theme_is_fixed_and_uses_technical_instrument_values() {
        let theme = pump_theme();
        assert_eq!(theme, pump_theme());
        assert_eq!(theme.clear, PumpColor::rgb(39, 43, 40));
        assert_eq!(theme.accent_mint, PumpColor::rgb(233, 107, 80));
        assert_eq!(theme.accent_copper, PumpColor::rgb(233, 107, 80));
        assert_eq!(theme.text_primary, PumpColor::rgb(213, 216, 214));
    }

    #[test]
    fn metrics_and_typography_match_the_visual_contract() {
        assert_eq!(PUMP_VISUAL_METRICS.base, 4.0);
        assert_eq!(PUMP_VISUAL_METRICS.control_height, 24.0);
        assert_eq!(PUMP_VISUAL_METRICS.knob, 32.0);
        assert_eq!(PUMP_VISUAL_METRICS.deck_height, 80.0);
        assert_eq!(PUMP_TYPOGRAPHY.brand, (17.0, 24.0));
        assert_eq!(PUMP_TYPOGRAPHY.meta, (8.0, 10.0));
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
