use super::*;
impl GuiRuntime {
    pub(super) fn new() -> Self {
        Self {
            selected_node: None,
            selected_nodes: Vec::new(),
            drag_mode: None,
            marquee_selection: None,
            curve_hovered: false,
            curve_local_pointer: Point { x: 0, y: 0 },
            curve_size: UiLayoutMetrics::design_space().curve_size,
            snap_enabled: false,
            snap_hovered: false,
            grid_override: None,
            shortcut_snap_invert_held: false,
            preset_rename_active: false,
            preset_rename_target: 0,
            preset_name_draft: String::new(),
            preset_warning_frames: 0,
            preset_warning_text: None,
            quick_slot_hovered: None,
            quick_slot_pressed: None,
            loaded_global_curve_slot: None,
            pointer_primary_down: false,
            pointer_secondary_down: false,
            active_knob_gesture_param: None,
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            knob_history_anchor: None,
            curve_history_anchor: None,
        }
    }
}

impl GuiState {
    pub(super) fn new(
        params: Arc<PumpParams>,
        status: Arc<GuiStatus>,
        automation_queue: Arc<AutomationQueue>,
        param_requester: Option<HostParamRequester>,
    ) -> Self {
        Self {
            params,
            status,
            automation_queue,
            automation_config: AutomationConfig::default(),
            param_requester,
            runtime: Mutex::new(GuiRuntime::new()),
        }
    }

    /// Snapshot current plugin control values for UI rendering.
    pub(super) fn snapshot_controls(&self) -> ControlSnapshot {
        let (snap_enabled, snap_hovered, grid_override, shortcut_snap_invert_held) = self
            .runtime
            .lock()
            .map(|runtime| {
                (
                    runtime.snap_enabled,
                    runtime.snap_hovered,
                    runtime.grid_override,
                    runtime.shortcut_snap_invert_held,
                )
            })
            .unwrap_or((false, false, None, false));
        ControlSnapshot {
            mix: self.params.mix(),
            phase_offset: self.params.phase_offset(),
            output_gain_db: self.params.output_gain_db(),
            division: self.params.sync_division(),
            incoming_waveform_enabled: self.status.incoming_waveform_enabled(),
            snap_enabled,
            snap_hovered,
            grid_override,
            shortcut_snap_invert_held,
        }
    }

    /// Snapshot preset-bank state and transient header interaction flags.
    pub(super) fn snapshot_presets(&self) -> PresetSnapshot {
        let bank = self.params.preset_bank_snapshot();
        let mut names = if bank.presets.is_empty() {
            vec![DEFAULT_PRESET_NAME.to_string()]
        } else {
            bank.presets
                .iter()
                .map(|preset| preset.name.clone())
                .collect()
        };
        let selected = bank.selected.min(names.len().saturating_sub(1));
        let persistence_warning = self.params.preset_persistence_warning().is_some();
        if persistence_warning {
            names[selected] = PRESET_WARNING_STORAGE.to_string();
        }
        let dirty = self.params.current_state_differs_from_selected_preset();

        let mut rename_active = false;
        let mut rename_draft = String::new();
        let mut warning_blink_visible = false;
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.preset_rename_active {
                runtime.preset_rename_target = runtime
                    .preset_rename_target
                    .min(names.len().saturating_sub(1));
                rename_active = true;
                rename_draft = runtime.preset_name_draft.clone();
            }
            if runtime.preset_warning_frames > 0 {
                warning_blink_visible = runtime.preset_warning_text.is_some()
                    && (runtime.preset_warning_frames / PRESET_WARNING_BLINK_HALF_PERIOD_FRAMES)
                        % 2
                        == 1;
                runtime.preset_warning_frames = runtime.preset_warning_frames.saturating_sub(1);
            }
        }

        PresetSnapshot {
            names,
            selected,
            dirty,
            rename_active,
            rename_draft,
            persistence_warning,
            warning_blink_visible,
        }
    }

    pub(super) fn set_preset_warning(&self, text: &'static str) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.preset_warning_text = Some(text);
            runtime.preset_warning_frames = PRESET_WARNING_FRAMES;
        }
    }

    /// Build the top header slot node.
    pub(super) fn build_header_slot(
        &self,
        metrics: UiLayoutMetrics,
        theme: PumpTheme,
        presets: &PresetSnapshot,
    ) -> Node {
        let header_h = resolve_vertical_slot_heights(metrics.content_h).0.max(1);
        let right_section_percent = if presets.persistence_warning {
            HEADER_STORAGE_WARNING_SECTION_PERCENT
        } else {
            HEADER_INDICATOR_SECTION_PERCENT
        };
        let left_section_percent = if presets.persistence_warning {
            100_u8.saturating_sub(HEADER_STORAGE_WARNING_SECTION_PERCENT)
        } else {
            HEADER_EMPTY_SECTION_PERCENT
        };
        let header_slot_widths = weighted_slot_lengths(
            metrics.content_w.max(1),
            &[left_section_percent as u16, right_section_percent as u16],
        );
        let left_width = header_slot_widths.first().copied().unwrap_or(1).max(1);
        let right_width = header_slot_widths.get(1).copied().unwrap_or(1).max(1);
        let action_button_width = (left_width / 8).max(metrics.transport_indicator_size.max(1));
        let preset_title_width = left_width.saturating_sub(action_button_width).max(1);
        let preset_selected_row_highlight = (presets.dirty || presets.warning_blink_visible)
            .then_some(theme.preset_dirty_highlight);
        let indicator_node = Node::align_box(
            indicator(
                Size {
                    width: metrics.transport_indicator_size,
                    height: metrics.transport_indicator_size,
                },
                self.status.transport_beat_blink_active(),
            )
            .widget_layout(fixed_box(
                metrics.transport_indicator_size,
                metrics.transport_indicator_size,
            )),
        )
        .slot_align(SlotAlign::Center, SlotAlign::Center)
        .fill();
        let status_label = Node::align_box(
            textbox(if presets.persistence_warning {
                PRESET_WARNING_STORAGE.to_string()
            } else {
                build_version_label()
            })
            .text_color(if presets.persistence_warning {
                theme.preset_dirty_highlight
            } else {
                theme.version_label
            })
            .text_align_center()
            .widget_layout(fixed_box(
                right_width,
                HEADER_VERSION_LABEL_HEIGHT.min(header_h).max(1),
            )),
        )
        .slot_align(SlotAlign::Center, SlotAlign::Center)
        .fill();
        let right_status = column_slots(vec![
            weighted_slot(status_label, 8),
            weighted_slot(indicator_node, 11),
        ])
        .container_overflow(OverflowPolicy::Compress)
        .fill();

        let preset_dropdown_or_edit = if presets.rename_active {
            textbox(presets.rename_draft.clone())
                .text_editable(PRESET_RENAME_KEY, true)
                .text_edit_max_chars(MAX_PRESET_NAME_CHARS)
                .widget_layout(LayoutBox::fill())
                .fill()
        } else {
            let mut preset_dropdown = dropdown(
                PRESET_DROPDOWN_KEY,
                presets.names.len().max(1),
                presets.selected.min(presets.names.len().saturating_sub(1)),
            )
            .dropdown_option_labels(presets.names.clone())
            .control_size(Size {
                width: preset_title_width,
                height: header_h,
            })
            .fill();
            if let Some(highlight_color) = preset_selected_row_highlight {
                preset_dropdown =
                    preset_dropdown.dropdown_selected_option_background_color(highlight_color);
            }
            preset_dropdown
        };
        let preset_title = panel("preset-title", preset_dropdown_or_edit.fill())
            .pad_all(0)
            .fill();

        let action_button_slot = |node: Node| {
            Slot::with_params(
                node,
                SlotParams::intrinsic()
                    .cross_size(SlotCrossSize::Intrinsic)
                    .align(SlotAlign::Start, SlotAlign::Center),
            )
        };

        let rename_button = button(PRESET_RENAME_BUTTON_KEY)
            .button_label("R")
            .control_size(Size {
                width: action_button_width,
                height: header_h,
            })
            .fill();
        let save_button = button(PRESET_SAVE_KEY)
            .button_label("S")
            .control_size(Size {
                width: action_button_width,
                height: header_h,
            })
            .fill();
        let add_button = button(PRESET_ADD_KEY)
            .button_label("+")
            .control_size(Size {
                width: action_button_width,
                height: header_h,
            })
            .fill();
        let action_buttons = row_slots(vec![
            action_button_slot(rename_button),
            action_button_slot(save_button),
            action_button_slot(add_button),
            weighted_slot(
                spacer(Size {
                    width: 1,
                    height: 1,
                }),
                1,
            ),
        ])
        .container_overflow(OverflowPolicy::Compress)
        .fill();
        let left_controls = row_slots(vec![
            weighted_slot(preset_title, 82),
            weighted_slot(action_buttons, 18),
        ])
        .container_overflow(OverflowPolicy::Compress)
        .fill();
        let header_content = row_slots(vec![
            weighted_slot(left_controls, left_section_percent as u16),
            weighted_slot(right_status, right_section_percent as u16),
        ])
        .container_overflow(OverflowPolicy::Compress);
        panel("header", header_content).pad_all(0)
    }

    /// Build the spline/curve slot node.
    pub(super) fn build_spline_slot(
        &self,
        metrics: UiLayoutMetrics,
        theme: PumpTheme,
        controls: ControlSnapshot,
        command_down: bool,
    ) -> Node {
        let editable_curve = self.params.editable_curve_snapshot();
        let effective_grid = effective_grid_division(controls.division, controls.grid_override);
        let mut curve_style = curve_editor_style(theme);
        curve_style.background = Color::rgba(0, 0, 0, 0);
        curve_style.grid_vertical = Color::rgba(0, 0, 0, 0);
        curve_style.grid_vertical_emphasis = Color::rgba(0, 0, 0, 0);
        curve_style.grid_horizontal = Color::rgba(0, 0, 0, 0);
        let mut curve_interaction = curve_editor_interaction_options(
            metrics.curve_size,
            effective_grid,
            controls.effective_snap_enabled(),
            command_down,
        );
        curve_interaction.whole_curve_offset = true;
        let curve_editor_view = curve_editor(CURVE_KEY, curve_model_from_editable(&editable_curve))
            .curve_style(curve_style)
            .curve_grid(curve_editor_grid_config(effective_grid))
            .curve_interaction(curve_interaction)
            .curve_segment_move(CurveSegmentMoveOptions::new(
                CurveEditorModifier::Command,
                theme.curve_segment_move,
            ))
            .curve_point_horizontal_constraint(CurveEditorModifier::Shift)
            .curve_point_vertical_constraint(CurveEditorModifier::ShiftOption)
            .curve_playhead_x(
                (self.status.has_host_beats_timeline() || self.status.is_playing())
                    .then_some(self.status.phase()),
            )
            .fill();
        let waveform = controls
            .incoming_waveform_enabled
            .then(|| self.status.incoming_waveform_snapshot())
            .flatten();
        let curve_viewport = stack(vec![
            surface(
                "incoming-waveform-underlay",
                metrics.curve_size,
                incoming_waveform_underlay_commands(metrics.curve_size, theme, waveform),
            )
            .fill(),
            surface(
                "curve-beat-grid",
                metrics.curve_size,
                curve_beat_grid_commands(metrics.curve_size, theme, effective_grid),
            )
            .fill(),
            surface(
                "curve-gain-references",
                metrics.curve_size,
                curve_gain_reference_line_commands(metrics.curve_size, theme),
            )
            .fill(),
            curve_editor_view,
        ])
        .fill();
        let reference_gutter_size = Size {
            width: metrics.curve_reference_gutter_width,
            height: metrics.curve_size.height,
        };
        let curve_content = row_slots(vec![
            weighted_slot(
                surface(
                    "curve-gain-reference-labels",
                    reference_gutter_size,
                    curve_gain_reference_label_commands(
                        reference_gutter_size,
                        theme,
                        metrics.text_scale,
                    ),
                )
                .fill(),
                metrics.curve_reference_gutter_width as u16,
            ),
            weighted_slot(curve_viewport, metrics.curve_size.width as u16),
        ])
        .container_overflow(OverflowPolicy::Compress)
        .fill();
        let meter_db = self.status.gain_reduction_db();
        let meter_bar_height = metrics
            .curve_size
            .height
            .saturating_sub(metrics.label_line_h.saturating_mul(2));
        let meter_value = if meter_db < 0.5 {
            "0".to_string()
        } else {
            format!("{meter_db:.0}")
        };
        let meter_panel = column(vec![
            textbox("dB")
                .text_align_center()
                .text_color(theme.version_label)
                .widget_layout(fixed_box(metrics.meter_panel_width, metrics.label_line_h)),
            surface(
                "gain-reduction-meter",
                Size {
                    width: metrics.meter_panel_width,
                    height: meter_bar_height,
                },
                gain_reduction_meter_commands(
                    Size {
                        width: metrics.meter_panel_width,
                        height: meter_bar_height,
                    },
                    theme,
                    meter_db,
                ),
            ),
            textbox(meter_value)
                .text_align_center()
                .text_color(theme.meter_fill)
                .widget_layout(fixed_box(metrics.meter_panel_width, metrics.label_line_h)),
        ])
        .container_overflow(OverflowPolicy::Compress)
        .fill();
        let curve_and_meter = row_slots(vec![
            weighted_slot(curve_content, CURVE_EDITOR_SECTION_WEIGHT),
            weighted_slot(meter_panel, METER_SECTION_WEIGHT),
        ])
        .container_overflow(OverflowPolicy::Compress)
        .fill();
        let spline_content = Node::padding_box(curve_and_meter)
            .pad_xy(0, CURVE_VERTICAL_MARGIN as i32)
            .widget_layout(fixed_box(metrics.content_w, metrics.curve_h))
            .fill();
        panel("spline", spline_content).pad_all(0)
    }

    fn quick_slot_key(index: usize) -> String {
        format!("{QUICK_SLOT_KEY_PREFIX}{index}")
    }

    fn quick_slot_index_from_key(key: &str) -> Option<usize> {
        key.strip_prefix(QUICK_SLOT_KEY_PREFIX)
            .and_then(|suffix| suffix.parse::<usize>().ok())
            .filter(|index| *index < GLOBAL_CURVE_SLOT_COUNT)
    }

    pub(super) fn build_quick_slot_draw_commands(
        curve: Option<&EditableCurve>,
        size: Size,
        theme: PumpTheme,
        hovered: bool,
        active: bool,
        store_hovered: bool,
        deviated: bool,
    ) -> Vec<SurfaceCommand> {
        let rect = Rect {
            origin: Point { x: 0, y: 0 },
            size,
        };
        let fill = if deviated {
            theme.quick_slot_deviation_bg
        } else if active {
            theme.quick_slot_active_bg
        } else if store_hovered {
            theme.quick_slot_store_hover_bg
        } else if hovered {
            theme.quick_slot_hover_bg
        } else {
            theme.quick_slot_bg
        };
        let outline = if deviated {
            theme.quick_slot_outline_deviation
        } else if store_hovered {
            theme.quick_slot_outline_store_hover
        } else if hovered || active {
            theme.quick_slot_outline_hover
        } else {
            theme.quick_slot_outline
        };
        let margin = QUICK_SLOT_PREVIEW_MARGIN
            .min((size.width as i32).saturating_div(4))
            .min((size.height as i32).saturating_div(4))
            .max(1);
        let inner_w = (size.width as i32 - margin * 2).max(1);
        let inner_h = (size.height as i32 - margin * 2).max(1);
        let steps = QUICK_SLOT_PREVIEW_STEPS.max(2);
        let points = if let Some(curve) = curve {
            (0..steps)
                .map(|step| {
                    let t = if steps <= 1 {
                        0.0
                    } else {
                        step as f32 / (steps - 1) as f32
                    };
                    let x = margin + (t * inner_w as f32).round() as i32;
                    let y = margin
                        + ((1.0 - sample_editable_curve(curve, t).clamp(0.0, 1.0)) * inner_h as f32)
                            .round() as i32;
                    Point { x, y }
                })
                .collect()
        } else {
            let y = margin + inner_h / 2;
            vec![
                Point { x: margin, y },
                Point {
                    x: margin + inner_w,
                    y,
                },
            ]
        };
        vec![
            SurfaceCommand::FillRect { rect, color: fill },
            SurfaceCommand::StrokeRect {
                rect,
                thickness: 1,
                color: outline,
            },
            SurfaceCommand::Polyline {
                points,
                thickness: if hovered || active || deviated {
                    2.0
                } else {
                    1.0
                },
                color: if deviated {
                    theme.quick_slot_deviation_curve
                } else if curve.is_some() {
                    theme.quick_slot_curve
                } else {
                    theme.quick_slot_empty_curve
                },
            },
        ]
    }

    fn quick_slot_surface(
        index: usize,
        curve: Option<&EditableCurve>,
        size: Size,
        theme: PumpTheme,
        visual: QuickSlotVisualState,
    ) -> Node {
        Node::align_box(surface(
            Self::quick_slot_key(index),
            size,
            Self::build_quick_slot_draw_commands(
                curve,
                size,
                theme,
                visual.hovered,
                visual.active,
                visual.store_hovered,
                visual.deviated,
            ),
        ))
        .slot_align(SlotAlign::Start, SlotAlign::Center)
        .fill()
    }

    pub(super) fn build_snap_checkbox_draw_commands(
        size: Size,
        theme: PumpTheme,
        enabled: bool,
        hovered: bool,
    ) -> Vec<SurfaceCommand> {
        let rect = Rect {
            origin: Point { x: 0, y: 0 },
            size,
        };
        let fill = if enabled {
            theme.snap_checkbox_active_bg
        } else if hovered {
            theme.snap_checkbox_hover_bg
        } else {
            theme.snap_checkbox_bg
        };
        let outline = if hovered || enabled {
            theme.snap_checkbox_outline_hover
        } else {
            theme.snap_checkbox_outline
        };
        vec![
            SurfaceCommand::FillRect { rect, color: fill },
            SurfaceCommand::StrokeRect {
                rect,
                thickness: 1,
                color: outline,
            },
        ]
    }

    fn snap_checkbox_surface(size: Size, theme: PumpTheme, enabled: bool, hovered: bool) -> Node {
        Node::align_box(surface(
            SNAP_KEY,
            size,
            Self::build_snap_checkbox_draw_commands(size, theme, enabled, hovered),
        ))
        .slot_align(SlotAlign::End, SlotAlign::Center)
        .fill()
    }

    /// Build the quick-slot strip shown below the curve editor.
    pub(super) fn build_quick_shapes_slot(
        &self,
        metrics: UiLayoutMetrics,
        theme: PumpTheme,
        command_down: bool,
    ) -> Node {
        let quick_slots = self.params.global_curve_slots_snapshot();
        let (hovered_slot, pressed_slot, loaded_slot) = self
            .runtime
            .lock()
            .ok()
            .map(|runtime| {
                (
                    runtime.quick_slot_hovered,
                    runtime.quick_slot_pressed,
                    runtime.loaded_global_curve_slot,
                )
            })
            .unwrap_or((None, None, None));
        let deviated_slot =
            loaded_slot.filter(|index| self.params.current_curve_deviates_from_global_slot(*index));
        let buttons = quick_slots
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                let size = Size {
                    width: metrics.quick_shape_button_w.max(1),
                    height: metrics.quick_shape_button_h,
                };
                weighted_slot(
                    Self::quick_slot_surface(
                        index,
                        slot.curve.as_ref(),
                        size,
                        theme,
                        QuickSlotVisualState {
                            hovered: hovered_slot == Some(index),
                            active: pressed_slot == Some(index) || loaded_slot == Some(index),
                            store_hovered: hovered_slot == Some(index) && command_down,
                            deviated: deviated_slot == Some(index),
                        },
                    ),
                    1,
                )
            })
            .collect();
        let quick_shapes_row = row_slots(buttons)
            .container_overflow(OverflowPolicy::Compress)
            .fill();
        panel("quick-shapes", quick_shapes_row).pad_all(0)
    }

    /// Build the controls slot node.
    pub(super) fn build_controls_slot(
        &self,
        metrics: UiLayoutMetrics,
        theme: PumpTheme,
        controls: ControlSnapshot,
    ) -> Node {
        const KNOB_TEXT_MAX_CHARS: u32 = 8;
        const MONO_CHAR_CELL_WIDTH_PX: u32 = 6;
        let snap_checkbox_size = metrics.button_control_h.saturating_sub(8).max(12);
        let knob_text_scale = metrics
            .knob_track_w
            .saturating_div(KNOB_TEXT_MAX_CHARS.saturating_mul(MONO_CHAR_CELL_WIDTH_PX))
            .max(1)
            .min(metrics.text_scale.max(1));
        let knob_label_h = metrics
            .label_line_h
            .max(scaled_line_height(knob_text_scale));
        let knob_cell = |key: &'static str,
                         label: &'static str,
                         value: f32,
                         default_value: f32,
                         range: (f32, f32),
                         value_text: String| {
            let title = Node::align_box(
                textbox(label)
                    .text_align_center()
                    .widget_layout(fixed_box(metrics.knob_track_w, knob_label_h)),
            )
            .slot_align(SlotAlign::Center, SlotAlign::Start)
            .fill();
            let value_label = Node::align_box(
                textbox(value_text)
                    .text_align_center()
                    .widget_layout(fixed_box(metrics.knob_track_w, knob_label_h)),
            )
            .slot_align(SlotAlign::Center, SlotAlign::Start)
            .fill();
            let knob_body = Node::align_box(
                knob(key, value, range)
                    .default_value(default_value)
                    .control_size(Size {
                        width: metrics.knob_diameter,
                        height: metrics.knob_diameter,
                    })
                    .widget_layout(fixed_box(metrics.knob_diameter, metrics.knob_diameter)),
            )
            .slot_align(SlotAlign::Center, SlotAlign::Center)
            .fill();
            column_slots(vec![
                weighted_slot(title, 15),
                weighted_slot(knob_body, 70),
                weighted_slot(value_label, 15),
            ])
            .container_overflow(OverflowPolicy::Compress)
        };
        let knobs_grid = grid(
            GridTemplate::new(vec![TrackSize::Auto; KNOBS_PER_ROW])
                .rows(vec![TrackSize::Auto])
                .justify_start(),
            vec![
                knob_cell(
                    MIX_KEY,
                    "Mix",
                    controls.mix,
                    DEFAULT_MIX,
                    (MIN_MIX, MAX_MIX),
                    format!("{:.0}%", controls.mix * 100.0),
                ),
                knob_cell(
                    PHASE_KEY,
                    "Phase",
                    controls.phase_offset,
                    DEFAULT_PHASE_OFFSET,
                    (MIN_PHASE_OFFSET, MAX_PHASE_OFFSET),
                    format!("{:.0}%", controls.phase_offset * 100.0),
                ),
                knob_cell(
                    OUTPUT_KEY,
                    "Output",
                    controls.output_gain_db,
                    DEFAULT_OUTPUT_GAIN_DB,
                    (MIN_OUTPUT_GAIN_DB, MAX_OUTPUT_GAIN_DB),
                    format!("{:+.0}dB", controls.output_gain_db),
                ),
            ],
        )
        .fill()
        .container_overflow(OverflowPolicy::Compress);

        let knobs_slot = panel("knobs", knobs_grid.fill()).pad_all(0);
        let snap_row = row_slots(vec![
            weighted_slot(
                Node::align_box(
                    textbox("Snap").widget_layout(fixed_box(
                        metrics
                            .dropdown_control_w
                            .saturating_sub(snap_checkbox_size.saturating_add(8))
                            .max(1),
                        metrics.button_control_h,
                    )),
                )
                .slot_align(SlotAlign::Start, SlotAlign::Center)
                .fill(),
                2,
            ),
            weighted_slot(
                Self::snap_checkbox_surface(
                    Size {
                        width: snap_checkbox_size,
                        height: snap_checkbox_size,
                    },
                    theme,
                    controls.snap_enabled,
                    controls.snap_hovered,
                ),
                1,
            ),
        ])
        .container_overflow(OverflowPolicy::Compress)
        .fill();
        let waveform_row = row_slots(vec![
            weighted_slot(
                Node::align_box(
                    textbox("Wave").widget_layout(fixed_box(
                        metrics
                            .dropdown_control_w
                            .saturating_sub(snap_checkbox_size.saturating_add(8))
                            .max(1),
                        metrics.button_control_h,
                    )),
                )
                .slot_align(SlotAlign::Start, SlotAlign::Center)
                .fill(),
                2,
            ),
            weighted_slot(
                toggle(INCOMING_WAVEFORM_KEY, controls.incoming_waveform_enabled)
                    .control_size(Size {
                        width: snap_checkbox_size,
                        height: snap_checkbox_size,
                    })
                    .widget_layout(fixed_box(snap_checkbox_size, snap_checkbox_size)),
                1,
            ),
        ])
        .container_overflow(OverflowPolicy::Compress)
        .fill();
        let dropdown_slot_content = column(vec![
            dropdown(
                DIVISION_KEY,
                MAX_SYNC_DIVISION as usize + 1,
                controls.division.min(MAX_SYNC_DIVISION as usize),
            )
            .dropdown_option_labels(
                (0..=MAX_SYNC_DIVISION as usize)
                    .map(|index| sync_division_label(index).to_string())
                    .collect(),
            )
            .control_size(Size {
                width: metrics.dropdown_control_w,
                height: metrics.dropdown_control_h,
            })
            .fill(),
            row_slots(vec![
                weighted_slot(snap_row, 2),
                weighted_slot(
                    dropdown(
                        GRID_OVERRIDE_KEY,
                        MAX_SYNC_DIVISION as usize + 2,
                        controls
                            .grid_override
                            .map(|index| index.saturating_add(1))
                            .unwrap_or(0),
                    )
                    .dropdown_option_labels(grid_override_option_labels())
                    .control_size(Size {
                        width: metrics
                            .dropdown_control_w
                            .saturating_mul(2)
                            .saturating_div(3),
                        height: metrics.button_control_h,
                    })
                    .fill(),
                    3,
                ),
            ])
            .container_overflow(OverflowPolicy::Compress)
            .fill(),
            waveform_row,
        ])
        .pad_all(0)
        .fill()
        .container_overflow(OverflowPolicy::Compress);
        let dropdown_slot = panel("dropdown", dropdown_slot_content.fill()).pad_all(0);

        let controls_row = row_slots(vec![
            weighted_slot(knobs_slot, KNOBS_SECTION_WEIGHT),
            weighted_slot(dropdown_slot, DROPDOWN_SECTION_WEIGHT)
                .align(SlotAlign::End, SlotAlign::Start),
        ])
        .container_overflow(OverflowPolicy::Compress);
        panel("controls", controls_row).pad_all(0)
    }

    /// Build the root UI spec for the current frame dimensions and content tree.
    pub(super) fn build_root_spec(
        &self,
        metrics: UiLayoutMetrics,
        theme: PumpTheme,
        content: Node,
    ) -> UiSpec {
        let design_size = Size {
            width: metrics.content_w,
            height: metrics.content_h,
        };
        UiSpec::new(
            root_frame_sized(ROOT_KEY, content, design_size)
                .padding(0)
                .scale_mode(RootScaleMode::UniformFit)
                .tokens(theme.tokens),
        )
    }

    pub(super) fn build_ui(&self, input: &InputState) -> UiSpec {
        let metrics = UiLayoutMetrics::design_space();
        self.sync_knob_gesture_state(input.mouse_down, input.mouse_secondary_down);
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.shortcut_snap_invert_held = input.shortcut_key_down(SHORTCUT_KEY_SNAP_INVERT);
            runtime.curve_size = metrics.curve_size;
        }
        let theme = PumpTheme::main(metrics);
        let controls = self.snapshot_controls();
        let presets = self.snapshot_presets();

        let header_slot = self.build_header_slot(metrics, theme, &presets);
        let spline_slot = self.build_spline_slot(metrics, theme, controls, input.command_down);
        let quick_shapes_slot = self.build_quick_shapes_slot(metrics, theme, input.command_down);
        let controls_slot = self.build_controls_slot(metrics, theme, controls);

        let content = column_slots(vec![
            weighted_slot(header_slot, HEADER_SECTION_WEIGHT),
            weighted_slot(spline_slot, CURVE_SECTION_WEIGHT),
            weighted_slot(quick_shapes_slot, QUICK_SHAPES_SECTION_WEIGHT),
            weighted_slot(controls_slot, CONTROLS_SECTION_WEIGHT),
        ])
        .container_overflow(OverflowPolicy::Compress);
        self.build_root_spec(metrics, theme, content)
    }

    pub(super) fn measured_open_size(&self) -> (u32, u32) {
        // Open at baseline design size so initial rendering is true 1:1.
        (WINDOW_WIDTH, WINDOW_HEIGHT)
    }

    pub(super) fn reduce_action(&mut self, action: UiAction) {
        match action {
            UiAction::KnobChanged { key, value } => {
                if matches!(key.as_str(), MIX_KEY | PHASE_KEY | OUTPUT_KEY) {
                    self.capture_knob_undo_anchor();
                }
                self.reduce_knob(key.as_str(), value);
            }
            UiAction::DropdownSelected { key, index } => {
                if matches!(key.as_str(), PRESET_DROPDOWN_KEY | DIVISION_KEY) {
                    self.capture_undo_snapshot();
                }
                self.reduce_dropdown(key.as_str(), index);
            }
            UiAction::ToggleChanged { key, value } if key == SNAP_KEY => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.snap_enabled = value;
                }
            }
            UiAction::ToggleChanged { key, value } if key == INCOMING_WAVEFORM_KEY => {
                self.status.set_incoming_waveform_enabled(value);
            }
            UiAction::RegionHover { key, hovered, .. } if key == SNAP_KEY => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.snap_hovered = hovered;
                }
            }
            UiAction::ButtonPressed { key } if key == UNDO_KEY => {
                self.apply_undo();
            }
            UiAction::ButtonPressed { key } if key == REDO_KEY => {
                self.apply_redo();
            }
            UiAction::ButtonPressed { key } if key == PRESET_RENAME_BUTTON_KEY => {
                self.begin_preset_rename();
            }
            UiAction::ButtonPressed { key } if key == PRESET_ADD_KEY => {
                self.capture_undo_snapshot();
                match self.params.add_preset_from_current_state() {
                    Ok(_) => {
                        if let Ok(mut runtime) = self.runtime.lock() {
                            runtime.preset_rename_active = false;
                            runtime.preset_name_draft.clear();
                            runtime.preset_warning_frames = 0;
                            runtime.preset_warning_text = None;
                        }
                    }
                    Err(PresetMutationError::CapacityReached) => {
                        self.set_preset_warning(PRESET_WARNING_MAX)
                    }
                    Err(PresetMutationError::PersistenceFailed { .. }) => {
                        self.set_preset_warning(PRESET_WARNING_STORAGE)
                    }
                    Err(_) => self.set_preset_warning(PRESET_WARNING_NAME),
                }
            }
            UiAction::ButtonPressed { key } if key == PRESET_SAVE_KEY => {
                self.capture_undo_snapshot();
                self.save_current_preset_by_name();
            }
            UiAction::TextBoxEdited { key, text } if key == PRESET_RENAME_KEY => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.preset_name_draft = text;
                }
            }
            UiAction::TextBoxEditCommitted { key, text } if key == PRESET_RENAME_KEY => {
                self.capture_undo_snapshot();
                self.commit_preset_rename(text.as_str());
            }
            UiAction::TextBoxEditCanceled { key } if key == PRESET_RENAME_KEY => {
                self.cancel_preset_rename();
            }
            UiAction::CurveEditorChanged { key, model } if key == CURVE_KEY => {
                self.capture_curve_undo_anchor();
                let editable_curve = editable_curve_from_model(&model);
                self.params.set_editable_curve(&editable_curve);
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.drag_mode = None;
                    runtime.selected_node = None;
                    runtime.selected_nodes.clear();
                    runtime.marquee_selection = None;
                }
            }
            UiAction::RegionHover {
                key,
                hovered,
                local_pointer,
            } if key == CURVE_KEY => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.curve_hovered = hovered;
                    runtime.curve_local_pointer =
                        scale_point_to_design(local_pointer, runtime.curve_size);
                    let curve_local_pointer = runtime.curve_local_pointer;
                    let mut should_finalize_marquee = false;
                    if let Some(marquee) = runtime.marquee_selection.as_mut() {
                        marquee.current_pointer = curve_local_pointer;
                        should_finalize_marquee = !runtime.pointer_secondary_down;
                    }
                    if should_finalize_marquee {
                        self.finalize_curve_marquee_selection_locked(&mut runtime);
                    }
                }
            }
            UiAction::RegionHover { key, hovered, .. }
                if Self::quick_slot_index_from_key(key.as_str()).is_some() =>
            {
                if let Ok(mut runtime) = self.runtime.lock() {
                    let index = Self::quick_slot_index_from_key(key.as_str());
                    if hovered {
                        runtime.quick_slot_hovered = index;
                    } else if runtime.quick_slot_hovered == index {
                        runtime.quick_slot_hovered = None;
                    }
                }
            }
            UiAction::RegionHover { .. } => {}
            UiAction::RegionInteracted {
                key,
                kind: RegionInteractionKind::Pressed,
                ..
            } if key == SNAP_KEY => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.snap_enabled = !runtime.snap_enabled;
                }
            }
            UiAction::RegionInteracted {
                key,
                kind,
                local_pointer,
                raw_local_pointer,
                alt_down,
                command_down,
                ..
            } if key == CURVE_KEY => self.reduce_curve_interaction_with_modifiers(
                kind,
                local_pointer,
                raw_local_pointer,
                alt_down,
                command_down,
            ),
            UiAction::RegionInteracted {
                key,
                kind,
                command_down,
                ..
            } if Self::quick_slot_index_from_key(key.as_str()).is_some() => {
                if let Some(index) = Self::quick_slot_index_from_key(key.as_str()) {
                    self.reduce_quick_slot_interaction(index, kind, command_down);
                }
            }
            UiAction::RegionInteracted { .. } => {}
            _ => {}
        }
    }

    fn snapshot_history_state(&self) -> UiHistorySnapshot {
        UiHistorySnapshot {
            mix: self.params.mix(),
            phase_offset: self.params.phase_offset(),
            output_gain_db: self.params.output_gain_db(),
            sync_division: self.params.sync_division(),
            editable_curve: self.params.editable_curve_snapshot(),
            preset_bank: self.params.preset_bank_snapshot(),
        }
    }

    fn apply_history_state(&self, snapshot: &UiHistorySnapshot) -> bool {
        let current_bank = self.params.preset_bank_snapshot();
        if current_bank != snapshot.preset_bank
            && self
                .params
                .set_preset_bank(snapshot.preset_bank.clone())
                .is_err()
        {
            return false;
        }
        self.params.set_mix(snapshot.mix);
        self.params.set_phase_offset(snapshot.phase_offset);
        self.params.set_output_gain_db(snapshot.output_gain_db);
        self.params.set_sync_division(snapshot.sync_division as f32);
        self.params.set_editable_curve(&snapshot.editable_curve);
        self.push_all_param_updates();
        true
    }

    fn push_history_snapshot(stack: &mut Vec<UiHistorySnapshot>, snapshot: UiHistorySnapshot) {
        if stack.last() == Some(&snapshot) {
            return;
        }
        stack.push(snapshot);
        if stack.len() > HISTORY_STEP_LIMIT {
            stack.remove(0);
        }
    }

    fn push_undo_snapshot_locked(runtime: &mut GuiRuntime, snapshot: UiHistorySnapshot) {
        Self::push_history_snapshot(&mut runtime.undo_history, snapshot);
        runtime.redo_history.clear();
    }

    fn commit_curve_history_anchor_locked(runtime: &mut GuiRuntime) {
        if let Some(snapshot) = runtime.curve_history_anchor.take() {
            Self::push_undo_snapshot_locked(runtime, snapshot);
        }
    }

    fn commit_knob_history_anchor_locked(runtime: &mut GuiRuntime) {
        if let Some(snapshot) = runtime.knob_history_anchor.take() {
            Self::push_undo_snapshot_locked(runtime, snapshot);
        }
    }

    fn commit_history_anchors_locked(runtime: &mut GuiRuntime) {
        Self::commit_curve_history_anchor_locked(runtime);
        Self::commit_knob_history_anchor_locked(runtime);
    }

    fn capture_undo_snapshot(&self) {
        let snapshot = self.snapshot_history_state();
        if let Ok(mut runtime) = self.runtime.lock() {
            Self::commit_history_anchors_locked(&mut runtime);
            Self::push_undo_snapshot_locked(&mut runtime, snapshot);
            runtime.knob_history_anchor = None;
            runtime.curve_history_anchor = None;
        }
    }

    fn capture_knob_undo_anchor(&self) {
        let snapshot = self.snapshot_history_state();
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.pointer_primary_down {
                if runtime.knob_history_anchor.is_none() {
                    runtime.knob_history_anchor = Some(snapshot);
                }
            } else {
                Self::push_undo_snapshot_locked(&mut runtime, snapshot);
                runtime.knob_history_anchor = None;
            }
        }
    }

    fn capture_curve_undo_anchor(&self) {
        let snapshot = self.snapshot_history_state();
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.pointer_primary_down {
                if runtime.curve_history_anchor.is_none() {
                    runtime.curve_history_anchor = Some(snapshot);
                }
            } else {
                Self::push_undo_snapshot_locked(&mut runtime, snapshot);
                runtime.curve_history_anchor = None;
            }
        }
    }

    fn normalized_selected_indices(indices: &[usize], node_count: usize) -> Vec<usize> {
        let mut normalized: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|index| *index < node_count)
            .collect();
        normalized.sort_unstable();
        normalized.dedup();
        normalized
    }

    fn x_delta_bounds_for_selected_nodes(
        origin_curve: &EditableCurve,
        selected: &[usize],
    ) -> (f32, f32) {
        let node_count = origin_curve.nodes.len();
        if node_count <= 2 {
            return (0.0, 0.0);
        }
        let mut selected_mask = vec![false; node_count];
        for index in selected.iter().copied() {
            if index < node_count {
                selected_mask[index] = true;
            }
        }
        let mut min_delta = f32::NEG_INFINITY;
        let mut max_delta = f32::INFINITY;
        let mut has_movable = false;
        let last_index = node_count.saturating_sub(1);
        for index in 1..last_index {
            if !selected_mask[index] {
                continue;
            }
            has_movable = true;
            if !selected_mask[index - 1] {
                let min_x = origin_curve.nodes[index - 1].x + NODE_X_MIN_SPACING;
                min_delta = min_delta.max(min_x - origin_curve.nodes[index].x);
            }
            if !selected_mask[index + 1] {
                let max_x = origin_curve.nodes[index + 1].x - NODE_X_MIN_SPACING;
                max_delta = max_delta.min(max_x - origin_curve.nodes[index].x);
            }
        }
        if !has_movable || min_delta > max_delta {
            return (0.0, 0.0);
        }
        (min_delta, max_delta)
    }

    fn move_selected_nodes_from_origin_for_size(
        origin_curve: &EditableCurve,
        selected_indices: &[usize],
        start_pointer: Point,
        raw_local_pointer: Point,
        curve_size: Size,
    ) -> EditableCurve {
        let mut editable = origin_curve.clone();
        let node_count = editable.nodes.len();
        if node_count < 2 {
            return editable;
        }
        let curve_width = curve_size.width.max(2);
        let curve_height = curve_size.height.max(2);
        let requested_delta_x =
            (raw_local_pointer.x - start_pointer.x) as f32 / (curve_width - 1) as f32;
        let delta_y = (start_pointer.y - raw_local_pointer.y) as f32 / (curve_height - 1) as f32;
        let (min_delta_x, max_delta_x) =
            Self::x_delta_bounds_for_selected_nodes(origin_curve, selected_indices);
        let delta_x = requested_delta_x.clamp(min_delta_x, max_delta_x);
        let last_index = node_count.saturating_sub(1);
        for index in selected_indices.iter().copied() {
            let Some(origin_node) = origin_curve.nodes.get(index).copied() else {
                continue;
            };
            let x = if index == 0 {
                0.0
            } else if index == last_index {
                1.0
            } else {
                (origin_node.x + delta_x).clamp(0.0, 1.0)
            };
            let y = (origin_node.y + delta_y).clamp(0.0, 1.0);
            if let Some(node) = editable.nodes.get_mut(index) {
                *node = CurveNode { x, y };
            }
        }
        editable
    }

    fn marquee_selected_nodes_for_size(
        curve: &EditableCurve,
        start_pointer: Point,
        current_pointer: Point,
        curve_size: Size,
    ) -> Vec<usize> {
        let dx = (current_pointer.x - start_pointer.x).abs();
        let dy = (current_pointer.y - start_pointer.y).abs();
        if dx <= curve_drag_threshold_px(curve_size) && dy <= curve_drag_threshold_px(curve_size) {
            return find_node_hit_for_size(
                curve,
                current_pointer,
                node_hit_radius(curve_size),
                curve_size,
            )
            .into_iter()
            .collect();
        }
        let left = start_pointer.x.min(current_pointer.x);
        let right = start_pointer.x.max(current_pointer.x);
        let top = start_pointer.y.min(current_pointer.y);
        let bottom = start_pointer.y.max(current_pointer.y);
        curve
            .nodes
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(index, node)| {
                let local = local_from_node_for_size(node, curve_size);
                ((left..=right).contains(&local.x) && (top..=bottom).contains(&local.y))
                    .then_some(index)
            })
            .collect()
    }

    fn finalize_curve_marquee_selection_locked(&self, runtime: &mut GuiRuntime) {
        let Some(marquee) = runtime.marquee_selection.take() else {
            return;
        };
        let curve = self.params.editable_curve_snapshot();
        let selected = Self::marquee_selected_nodes_for_size(
            &curve,
            marquee.start_pointer,
            marquee.current_pointer,
            runtime.curve_size,
        );
        runtime.selected_nodes = selected.clone();
        runtime.selected_node = selected.last().copied();
    }

    fn apply_undo(&self) {
        let current = self.snapshot_history_state();
        let target = self
            .runtime
            .lock()
            .ok()
            .and_then(|runtime| runtime.undo_history.last().cloned());
        if let Some(snapshot) = target {
            if !self.apply_history_state(&snapshot) {
                return;
            }
            if let Ok(mut runtime) = self.runtime.lock() {
                runtime.undo_history.pop();
                Self::push_history_snapshot(&mut runtime.redo_history, current);
                runtime.knob_history_anchor = None;
                runtime.curve_history_anchor = None;
                runtime.drag_mode = None;
                runtime.selected_node = None;
                runtime.selected_nodes.clear();
                runtime.marquee_selection = None;
                runtime.quick_slot_pressed = None;
                runtime.preset_rename_active = false;
                runtime.preset_name_draft.clear();
                runtime.preset_warning_frames = 0;
                runtime.preset_warning_text = None;
            }
        }
    }

    fn apply_redo(&self) {
        let current = self.snapshot_history_state();
        let target = self
            .runtime
            .lock()
            .ok()
            .and_then(|runtime| runtime.redo_history.last().cloned());
        if let Some(snapshot) = target {
            if !self.apply_history_state(&snapshot) {
                return;
            }
            if let Ok(mut runtime) = self.runtime.lock() {
                runtime.redo_history.pop();
                Self::push_history_snapshot(&mut runtime.undo_history, current);
                runtime.knob_history_anchor = None;
                runtime.curve_history_anchor = None;
                runtime.drag_mode = None;
                runtime.selected_node = None;
                runtime.selected_nodes.clear();
                runtime.marquee_selection = None;
                runtime.quick_slot_pressed = None;
                runtime.preset_rename_active = false;
                runtime.preset_name_draft.clear();
                runtime.preset_warning_frames = 0;
                runtime.preset_warning_text = None;
            }
        }
    }

    pub(super) fn reduce_knob(&mut self, key: &str, value: f32) {
        let Some(param_id) = knob_param_id(key) else {
            return;
        };
        match key {
            MIX_KEY => {
                self.params.set_mix(value);
            }
            PHASE_KEY => {
                self.params.set_phase_offset(value);
            }
            OUTPUT_KEY => {
                self.params.set_output_gain_db(value);
            }
            _ => return,
        }
        self.push_knob_value_update(param_id, value as f64);
    }

    pub(super) fn reduce_dropdown(&mut self, key: &str, index: usize) {
        if key == PRESET_DROPDOWN_KEY {
            match self.params.load_preset(index) {
                Ok(selected) => {
                    self.push_all_param_updates();
                    if let Ok(mut runtime) = self.runtime.lock() {
                        runtime.preset_rename_active = false;
                        runtime.preset_name_draft.clear();
                        runtime.preset_rename_target = selected;
                        runtime.preset_warning_frames = 0;
                        runtime.preset_warning_text = None;
                    }
                }
                Err(PresetMutationError::PersistenceFailed { .. }) => {
                    self.set_preset_warning(PRESET_WARNING_STORAGE);
                }
                Err(_) => {}
            }
            return;
        }

        match key {
            DIVISION_KEY => {
                let clamped = index.min(MAX_SYNC_DIVISION as usize);
                self.params.set_sync_division(clamped as f32);
                self.push_single_value_update(PARAM_SYNC_DIVISION_ID, clamped as f64);
            }
            GRID_OVERRIDE_KEY => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.grid_override = match index {
                        0 => None,
                        selected => Some((selected - 1).min(MAX_SYNC_DIVISION as usize)),
                    };
                }
            }
            _ => {}
        }
    }

    pub(super) fn begin_preset_rename(&self) {
        let bank = self.params.preset_bank_snapshot();
        if bank.presets.is_empty() {
            return;
        }
        let selected = bank.selected.min(bank.presets.len().saturating_sub(1));
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.preset_rename_target = selected;
            runtime.preset_name_draft = bank.presets[runtime.preset_rename_target].name.clone();
            runtime.preset_rename_active = true;
            runtime.preset_warning_frames = 0;
            runtime.preset_warning_text = None;
        }
    }

    pub(super) fn commit_preset_rename(&self, text: &str) {
        let Ok(mut runtime) = self.runtime.lock() else {
            return;
        };
        if !runtime.preset_rename_active {
            return;
        }
        let target = runtime.preset_rename_target;
        let renamed = self.params.rename_preset(target, text);
        runtime.preset_rename_active = false;
        runtime.preset_name_draft.clear();
        if renamed.is_ok() {
            runtime.preset_warning_frames = 0;
            runtime.preset_warning_text = None;
        } else {
            runtime.preset_warning_text = Some(match renamed {
                Err(PresetMutationError::PersistenceFailed { .. }) => PRESET_WARNING_STORAGE,
                _ => PRESET_WARNING_NAME,
            });
            runtime.preset_warning_frames = PRESET_WARNING_FRAMES;
        }
    }

    pub(super) fn cancel_preset_rename(&self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.preset_rename_active = false;
            runtime.preset_name_draft.clear();
        }
    }

    pub(super) fn save_current_preset_by_name(&self) {
        let bank = self.params.preset_bank_snapshot();
        let selected = bank.selected.min(bank.presets.len().saturating_sub(1));
        let fallback_name = bank
            .presets
            .get(selected)
            .map(|preset| preset.name.clone())
            .unwrap_or_else(|| DEFAULT_PRESET_NAME.to_string());
        let candidate = self
            .runtime
            .lock()
            .ok()
            .and_then(|runtime| {
                if runtime.preset_rename_active {
                    Some(runtime.preset_name_draft.clone())
                } else {
                    None
                }
            })
            .unwrap_or(fallback_name);
        match self.params.save_current_state_by_name(&candidate) {
            Ok(SavePresetOutcome::Overwritten { index })
            | Ok(SavePresetOutcome::Created { index }) => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.preset_rename_active = false;
                    runtime.preset_name_draft.clear();
                    runtime.preset_rename_target = index;
                    runtime.preset_warning_frames = 0;
                    runtime.preset_warning_text = None;
                }
            }
            Err(PresetMutationError::CapacityReached) => {
                self.set_preset_warning(PRESET_WARNING_MAX)
            }
            Err(PresetMutationError::PersistenceFailed { .. }) => {
                self.set_preset_warning(PRESET_WARNING_STORAGE)
            }
            Err(_) => self.set_preset_warning(PRESET_WARNING_NAME),
        }
    }

    fn clear_curve_transient_state(&self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.selected_node = None;
            runtime.selected_nodes.clear();
            runtime.drag_mode = None;
            runtime.marquee_selection = None;
        }
    }

    /// Load one quick-slot curve into the main editable curve.
    pub(super) fn apply_quick_slot_curve(&self, index: usize) {
        let Some(curve) = self.params.global_curve_slot_curve(index) else {
            return;
        };
        self.params.set_editable_curve(&curve);
        self.clear_curve_transient_state();
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.loaded_global_curve_slot = Some(index);
        }
    }

    /// Store the current editable curve into one globally persisted quick slot.
    pub(super) fn store_quick_slot_curve(&self, index: usize) {
        let editable_curve = self.params.editable_curve_snapshot();
        if self
            .params
            .set_global_curve_slot_curve(index, &editable_curve)
        {
            if let Ok(mut runtime) = self.runtime.lock() {
                runtime.loaded_global_curve_slot = Some(index);
            }
        }
    }

    /// Apply one quick-slot interaction from the preview row.
    pub(super) fn reduce_quick_slot_interaction(
        &mut self,
        index: usize,
        kind: RegionInteractionKind,
        command_down: bool,
    ) {
        match kind {
            RegionInteractionKind::Pressed => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.quick_slot_pressed = Some(index);
                }
                if command_down {
                    self.store_quick_slot_curve(index);
                } else if self.params.global_curve_slot_curve(index).is_some() {
                    self.capture_undo_snapshot();
                    self.apply_quick_slot_curve(index);
                }
            }
            RegionInteractionKind::Released => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    if runtime.quick_slot_pressed == Some(index) {
                        runtime.quick_slot_pressed = None;
                    }
                }
            }
            RegionInteractionKind::Dragged
            | RegionInteractionKind::SecondaryClicked
            | RegionInteractionKind::DoubleClicked => {}
        }
    }

    #[cfg(test)]
    pub(super) fn reduce_curve_interaction(
        &mut self,
        kind: RegionInteractionKind,
        local_pointer: Point,
        raw_local_pointer: Point,
        alt_down: bool,
    ) {
        self.reduce_curve_interaction_with_modifiers(
            kind,
            local_pointer,
            raw_local_pointer,
            alt_down,
            false,
        );
    }

    fn reduce_curve_interaction_with_modifiers(
        &mut self,
        kind: RegionInteractionKind,
        local_pointer: Point,
        raw_local_pointer: Point,
        alt_down: bool,
        command_down: bool,
    ) {
        let Ok(mut runtime) = self.runtime.lock() else {
            return;
        };

        let local_pointer = scale_point_to_design(local_pointer, runtime.curve_size);
        let raw_local_pointer = scale_point_to_design(raw_local_pointer, runtime.curve_size);
        let normalized_pointer = node_from_local_for_size(local_pointer, runtime.curve_size);
        let raw_normalized_pointer =
            node_from_local_for_size(raw_local_pointer, runtime.curve_size);
        let command_snap_division =
            effective_grid_division(self.params.sync_division(), runtime.grid_override);
        let command_snap_width = runtime.curve_size.width as f32;

        match kind {
            RegionInteractionKind::Pressed => {
                let mut editable = self.params.editable_curve_snapshot();
                if let Some(index) = find_node_hit_for_size(
                    &editable,
                    local_pointer,
                    node_hit_radius(runtime.curve_size),
                    runtime.curve_size,
                ) {
                    let selected_nodes = Self::normalized_selected_indices(
                        &runtime.selected_nodes,
                        editable.nodes.len(),
                    );
                    let move_group = selected_nodes.len() > 1 && selected_nodes.contains(&index);
                    runtime.selected_node = Some(index);
                    runtime.selected_nodes = if move_group {
                        selected_nodes.clone()
                    } else {
                        vec![index]
                    };
                    runtime.drag_mode = Some(if move_group {
                        CurveDragMode::MoveNodeGroup {
                            origin_indices: selected_nodes,
                            origin_curve: editable.clone(),
                            start_pointer: local_pointer,
                            dragging: false,
                        }
                    } else {
                        CurveDragMode::MoveNode {
                            origin_index: index,
                            origin_curve: editable.clone(),
                            start_pointer: local_pointer,
                            dragging: false,
                        }
                    });
                    runtime.marquee_selection = None;
                    return;
                }

                let near_segment = find_segment_line_hit_within_for_size(
                    &editable,
                    local_pointer,
                    segment_near_hit_radius(runtime.curve_size),
                    runtime.curve_size,
                );

                if command_down && !alt_down {
                    if let Some(index) = near_segment {
                        let right_index = (index + 1).min(editable.nodes.len().saturating_sub(1));
                        runtime.drag_mode = Some(CurveDragMode::MoveSegment {
                            index,
                            start_pointer: local_pointer,
                            start_left_x: editable.nodes[index].x,
                            start_right_x: editable.nodes[right_index].x,
                            start_left_y: editable.nodes[index].y,
                            start_right_y: editable.nodes[right_index].y,
                            dragging: false,
                        });
                        runtime.selected_node = None;
                        runtime.selected_nodes.clear();
                        runtime.marquee_selection = None;
                        return;
                    }
                }

                if !alt_down
                    && find_segment_line_hit_within_for_size(
                        &editable,
                        local_pointer,
                        segment_direct_hit_radius(runtime.curve_size),
                        runtime.curve_size,
                    )
                    .is_some()
                {
                    let mut preview_node = preview_node_on_curve_for_size(
                        &editable,
                        local_pointer,
                        runtime.curve_size,
                    )
                    .unwrap_or(normalized_pointer);
                    if command_down {
                        preview_node.x = snap_curve_time_to_beat_grid(
                            command_snap_division,
                            command_snap_width,
                            preview_node.x,
                        );
                    }
                    let before_insert = editable.clone();
                    let inserted_index =
                        insert_node_for_size(&mut editable, preview_node, runtime.curve_size);
                    runtime.selected_node = Some(inserted_index);
                    runtime.selected_nodes = vec![inserted_index];
                    runtime.drag_mode = Some(CurveDragMode::MoveNode {
                        origin_index: inserted_index,
                        origin_curve: editable.clone(),
                        start_pointer: local_pointer,
                        dragging: false,
                    });
                    runtime.marquee_selection = None;
                    if editable != before_insert {
                        Self::push_undo_snapshot_locked(
                            &mut runtime,
                            self.snapshot_history_state(),
                        );
                        enforce_wrapped_endpoints(&mut editable);
                        self.params.set_editable_curve(&editable);
                    }
                    return;
                }

                if alt_down {
                    if let Some(index) = near_segment {
                        let start_tension = editable
                            .segments
                            .get(index)
                            .copied()
                            .unwrap_or(CurveSegment { tension: 0.0 })
                            .tension;
                        runtime.drag_mode = Some(CurveDragMode::AdjustSegmentCurve {
                            index,
                            start_pointer: local_pointer,
                            start_tension,
                            dragging: false,
                        });
                        runtime.selected_node = None;
                        runtime.selected_nodes.clear();
                        runtime.marquee_selection = None;
                        return;
                    }
                }

                if let Some(index) = find_node_hit_within_for_size(
                    &editable,
                    local_pointer,
                    node_insert_guard_radius(runtime.curve_size),
                    runtime.curve_size,
                ) {
                    runtime.selected_node = Some(index);
                    runtime.selected_nodes = vec![index];
                    runtime.drag_mode = Some(CurveDragMode::MoveNode {
                        origin_index: index,
                        origin_curve: editable.clone(),
                        start_pointer: local_pointer,
                        dragging: false,
                    });
                    runtime.marquee_selection = None;
                    return;
                }

                let mut insert_pointer = normalized_pointer;
                if command_down {
                    insert_pointer.x = snap_curve_time_to_beat_grid(
                        command_snap_division,
                        command_snap_width,
                        insert_pointer.x,
                    );
                }
                let before_insert = editable.clone();
                let inserted_index =
                    insert_node_for_size(&mut editable, insert_pointer, runtime.curve_size);
                runtime.selected_node = Some(inserted_index);
                runtime.selected_nodes = vec![inserted_index];
                runtime.drag_mode = Some(CurveDragMode::MoveNode {
                    origin_index: inserted_index,
                    origin_curve: editable.clone(),
                    start_pointer: local_pointer,
                    dragging: false,
                });
                runtime.marquee_selection = None;
                if editable != before_insert {
                    Self::push_undo_snapshot_locked(&mut runtime, self.snapshot_history_state());
                    enforce_wrapped_endpoints(&mut editable);
                    self.params.set_editable_curve(&editable);
                }
            }
            RegionInteractionKind::Dragged => {
                if let Some(mut drag_mode) = runtime.drag_mode.take() {
                    let mut editable = self.params.editable_curve_snapshot();
                    let mut curve_changed = false;
                    match drag_mode {
                        CurveDragMode::MoveNode {
                            origin_index,
                            origin_curve,
                            start_pointer,
                            mut dragging,
                        } => {
                            if !dragging
                                && !drag_threshold_crossed(
                                    start_pointer,
                                    local_pointer,
                                    curve_drag_threshold_px(runtime.curve_size),
                                )
                            {
                                runtime.drag_mode = Some(CurveDragMode::MoveNode {
                                    origin_index,
                                    origin_curve,
                                    start_pointer,
                                    dragging,
                                });
                                return;
                            }
                            dragging = true;
                            let mut target = raw_normalized_pointer;
                            if command_down {
                                target.x = snap_curve_time_to_beat_grid(
                                    command_snap_division,
                                    command_snap_width,
                                    target.x,
                                );
                            }
                            let (recomputed_curve, moved_index) =
                                recompute_move_node_from_origin_for_size(
                                    &origin_curve,
                                    origin_index,
                                    target,
                                    node_push_through_threshold_px(runtime.curve_size),
                                    runtime.curve_size,
                                );
                            editable = recomputed_curve;
                            runtime.selected_node = Some(moved_index);
                            runtime.selected_nodes = vec![moved_index];
                            drag_mode = CurveDragMode::MoveNode {
                                origin_index,
                                origin_curve,
                                start_pointer,
                                dragging,
                            };
                            curve_changed = true;
                        }
                        CurveDragMode::MoveNodeGroup {
                            origin_indices,
                            origin_curve,
                            start_pointer,
                            mut dragging,
                        } => {
                            if !dragging
                                && !drag_threshold_crossed(
                                    start_pointer,
                                    local_pointer,
                                    curve_drag_threshold_px(runtime.curve_size),
                                )
                            {
                                runtime.drag_mode = Some(CurveDragMode::MoveNodeGroup {
                                    origin_indices,
                                    origin_curve,
                                    start_pointer,
                                    dragging,
                                });
                                return;
                            }
                            dragging = true;
                            editable = Self::move_selected_nodes_from_origin_for_size(
                                &origin_curve,
                                &origin_indices,
                                start_pointer,
                                raw_local_pointer,
                                runtime.curve_size,
                            );
                            runtime.selected_nodes = origin_indices.clone();
                            drag_mode = CurveDragMode::MoveNodeGroup {
                                origin_indices,
                                origin_curve,
                                start_pointer,
                                dragging,
                            };
                            curve_changed = true;
                        }
                        CurveDragMode::MoveSegment {
                            index,
                            start_pointer,
                            start_left_x,
                            start_right_x,
                            start_left_y,
                            start_right_y,
                            mut dragging,
                        } => {
                            if !command_down {
                                return;
                            }
                            if !dragging
                                && !drag_threshold_crossed(
                                    start_pointer,
                                    local_pointer,
                                    curve_drag_threshold_px(runtime.curve_size),
                                )
                            {
                                runtime.drag_mode = Some(CurveDragMode::MoveSegment {
                                    index,
                                    start_pointer,
                                    start_left_x,
                                    start_right_x,
                                    start_left_y,
                                    start_right_y,
                                    dragging,
                                });
                                return;
                            }
                            dragging = true;
                            let curve_width = runtime.curve_size.width.max(2);
                            let curve_height = runtime.curve_size.height.max(2);
                            let delta_x = (raw_local_pointer.x - start_pointer.x) as f32
                                / (curve_width - 1) as f32;
                            let delta_y = (start_pointer.y - raw_local_pointer.y) as f32
                                / (curve_height - 1) as f32;
                            move_segment_translated(
                                &mut editable,
                                index,
                                (start_left_x, start_left_y),
                                (start_right_x, start_right_y),
                                (delta_x, delta_y),
                            );
                            drag_mode = CurveDragMode::MoveSegment {
                                index,
                                start_pointer,
                                start_left_x,
                                start_right_x,
                                start_left_y,
                                start_right_y,
                                dragging,
                            };
                            runtime.selected_node = None;
                            runtime.selected_nodes.clear();
                            curve_changed = true;
                        }
                        CurveDragMode::AdjustSegmentCurve {
                            index,
                            start_pointer,
                            start_tension,
                            mut dragging,
                        } => {
                            if !dragging
                                && !drag_threshold_crossed(
                                    start_pointer,
                                    local_pointer,
                                    curve_drag_threshold_px(runtime.curve_size),
                                )
                            {
                                runtime.drag_mode = Some(CurveDragMode::AdjustSegmentCurve {
                                    index,
                                    start_pointer,
                                    start_tension,
                                    dragging,
                                });
                                return;
                            }
                            dragging = true;
                            let delta = tension_delta_from_drag_for_segment(
                                &editable,
                                index,
                                start_pointer,
                                raw_local_pointer,
                                runtime.curve_size,
                            );
                            if let Some(segment) = editable.segments.get_mut(index) {
                                segment.tension = (start_tension + delta)
                                    .clamp(MIN_SEGMENT_TENSION, MAX_SEGMENT_TENSION);
                                curve_changed = true;
                            }
                            drag_mode = CurveDragMode::AdjustSegmentCurve {
                                index,
                                start_pointer,
                                start_tension,
                                dragging,
                            };
                            runtime.selected_node = None;
                            runtime.selected_nodes.clear();
                        }
                    }
                    runtime.drag_mode = Some(drag_mode);
                    if curve_changed {
                        if runtime.curve_history_anchor.is_none() {
                            runtime.curve_history_anchor = Some(self.snapshot_history_state());
                        }
                        enforce_wrapped_endpoints(&mut editable);
                        self.params.set_editable_curve(&editable);
                    }
                }
            }
            RegionInteractionKind::Released => {
                runtime.drag_mode = None;
                Self::commit_curve_history_anchor_locked(&mut runtime);
                runtime.curve_history_anchor = None;
            }
            RegionInteractionKind::SecondaryClicked => {
                runtime.drag_mode = None;
                runtime.marquee_selection = Some(CurveMarqueeSelection {
                    start_pointer: local_pointer,
                    current_pointer: local_pointer,
                });
                runtime.curve_history_anchor = None;
            }
            RegionInteractionKind::DoubleClicked => {
                let mut editable = self.params.editable_curve_snapshot();
                if let Some(index) =
                    find_deletable_node_hit_for_size(&editable, local_pointer, runtime.curve_size)
                {
                    editable.nodes.remove(index);
                    let remove_segment = index
                        .saturating_sub(1)
                        .min(editable.segments.len().saturating_sub(1));
                    if !editable.segments.is_empty() {
                        editable.segments.remove(remove_segment);
                    }
                    enforce_wrapped_endpoints(&mut editable);
                    runtime.selected_node = None;
                    runtime.selected_nodes.clear();
                    runtime.drag_mode = None;
                    runtime.marquee_selection = None;
                    runtime.curve_history_anchor = None;
                    Self::push_undo_snapshot_locked(&mut runtime, self.snapshot_history_state());
                    self.params.set_editable_curve(&editable);
                }
            }
        }
    }

    #[allow(dead_code)]
    pub(super) fn build_curve_draw_commands(
        &self,
        editable_curve: &EditableCurve,
        metrics: UiLayoutMetrics,
        state: CurveRenderState,
        theme: &PumpTheme,
    ) -> Vec<SurfaceCommand> {
        let curve_size = metrics.curve_size;
        let rect = Rect {
            origin: Point { x: 0, y: 0 },
            size: Size {
                width: curve_size.width,
                height: curve_size.height,
            },
        };
        let to_canvas = |point: Point| scale_point_from_design(point, curve_size);
        let border_stroke = scaled_curve_u32(METER_STROKE.max(0) as u32, curve_size);
        let node_radius = scaled_curve_i32(NODE_DRAW_RADIUS, curve_size);
        let node_hover_radius = scaled_curve_i32(NODE_DRAW_RADIUS + 1, curve_size);
        let node_preview_radius = scaled_curve_i32(NODE_DRAW_RADIUS + 1, curve_size);
        let node_preview_stroke_radius = scaled_curve_i32(NODE_DRAW_RADIUS + 2, curve_size);
        let node_ring_radius = scaled_curve_i32(NODE_DRAW_RADIUS + 3, curve_size);
        let playhead_core_radius = scaled_curve_i32(PLAYHEAD_DOT_CORE_RADIUS, curve_size);
        let playhead_glow_radius = scaled_curve_i32(PLAYHEAD_DOT_GLOW_RADIUS, curve_size);
        let node_stroke = scaled_curve_i32(METER_STROKE, curve_size);
        let highlight_offset = scaled_curve_i32(1, curve_size);

        let mut commands = Vec::with_capacity(1024);
        commands.push(SurfaceCommand::FillRect {
            rect,
            color: theme.curve_bg,
        });

        // SurfaceCommand does not expose polygon fills, so build a contiguous
        // low-alpha area from narrow strips. Sampling at two-pixel intervals
        // keeps the contour smooth without crowding the paint plan.
        const FILL_STRIP_WIDTH: u32 = 2;
        for x in (0..curve_size.width).step_by(FILL_STRIP_WIDTH as usize) {
            let phase = if curve_size.width > 1 {
                x as f32 / (curve_size.width - 1) as f32
            } else {
                0.0
            };
            let curve_point = to_canvas(local_from_node_for_size(
                CurveNode {
                    x: phase,
                    y: sample_editable_curve(editable_curve, phase),
                },
                curve_size,
            ));
            let top = curve_point
                .y
                .clamp(0, curve_size.height.saturating_sub(1) as i32);
            commands.push(SurfaceCommand::FillRect {
                rect: Rect {
                    origin: Point {
                        x: x as i32,
                        y: top,
                    },
                    size: Size {
                        width: FILL_STRIP_WIDTH.min(curve_size.width - x),
                        height: curve_size.height.saturating_sub(top as u32),
                    },
                },
                color: theme.curve_fill,
            });
        }
        commands.push(SurfaceCommand::StrokeRect {
            rect,
            thickness: border_stroke,
            color: theme.curve_border,
        });

        for step in 1..16 {
            let x = ((curve_size.width as i32 - 1) * step) / 16;
            commands.push(SurfaceCommand::Line {
                start: Point { x, y: 0 },
                end: Point {
                    x,
                    y: curve_size.height as i32 - 1,
                },
                color: theme.curve_grid_vertical,
            });
        }

        commands.extend(curve_gain_reference_line_commands(curve_size, *theme));

        for segment_index in 0..editable_curve.segments.len() {
            let left = editable_curve.nodes[segment_index];
            let right =
                editable_curve.nodes[(segment_index + 1).min(editable_curve.nodes.len() - 1)];
            let left_x = local_from_node_for_size(CurveNode { x: left.x, y: 0.0 }, curve_size).x;
            let right_x = local_from_node_for_size(CurveNode { x: right.x, y: 0.0 }, curve_size).x;
            let segment_width = (right_x - left_x).abs().max(2);
            let steps = segment_width.clamp(2, 96) as usize;
            let mut prev = to_canvas(local_from_node_for_size(
                CurveNode {
                    x: left.x,
                    y: sample_editable_curve(editable_curve, left.x),
                },
                curve_size,
            ));
            let highlight =
                state.preview_node.is_none() && state.hovered_segment == Some(segment_index);
            let line_color = if highlight {
                theme.curve_line_highlight
            } else {
                theme.curve_line
            };
            for step in 1..=steps {
                let t = step as f32 / steps as f32;
                let x = left.x + (right.x - left.x) * t;
                let point = to_canvas(local_from_node_for_size(
                    CurveNode {
                        x,
                        y: sample_editable_curve(editable_curve, x),
                    },
                    curve_size,
                ));
                commands.push(SurfaceCommand::Line {
                    start: prev,
                    end: point,
                    color: line_color,
                });
                if highlight {
                    commands.push(SurfaceCommand::Line {
                        start: Point {
                            x: prev.x,
                            y: prev.y + highlight_offset,
                        },
                        end: Point {
                            x: point.x,
                            y: point.y + highlight_offset,
                        },
                        color: theme.curve_line_highlight_glow,
                    });
                }
                prev = point;
            }
        }

        if let Some(preview) = state.preview_node {
            let center = to_canvas(local_from_node_for_size(preview, curve_size));
            commands.push(SurfaceCommand::FillCircle {
                center,
                radius: node_preview_radius,
                color: theme.preview_fill,
            });
            commands.push(SurfaceCommand::StrokeCircle {
                center,
                radius: node_preview_stroke_radius,
                thickness: node_stroke,
                color: theme.preview_stroke,
            });
        }

        for (index, node) in editable_curve.nodes.iter().copied().enumerate() {
            let center = to_canvas(local_from_node_for_size(node, curve_size));
            let selected =
                state.selected_node == Some(index) || state.selected_nodes.contains(&index);
            let hovered = state.hovered_node == Some(index);
            let fill_color = if selected {
                theme.node_selected_fill
            } else if hovered {
                theme.node_hover_fill
            } else {
                theme.node_fill
            };
            let stroke_color = if selected {
                theme.node_selected_stroke
            } else if hovered {
                theme.node_hover_stroke
            } else {
                theme.node_stroke
            };
            commands.push(SurfaceCommand::FillCircle {
                center,
                radius: if selected || hovered {
                    node_hover_radius
                } else {
                    node_radius
                },
                color: fill_color,
            });
            commands.push(SurfaceCommand::StrokeCircle {
                center,
                radius: node_radius,
                thickness: node_stroke,
                color: stroke_color,
            });
            if selected || hovered {
                commands.push(SurfaceCommand::StrokeCircle {
                    center,
                    radius: node_ring_radius,
                    thickness: node_stroke,
                    color: if selected {
                        theme.node_selected_ring
                    } else {
                        theme.node_hover_ring
                    },
                });
            }
        }

        if self.status.has_host_beats_timeline() || self.status.is_playing() {
            let phase = self.status.phase();
            let point = to_canvas(local_from_node_for_size(
                CurveNode {
                    x: phase,
                    y: sample_editable_curve(editable_curve, phase).clamp(0.0, 1.0),
                },
                curve_size,
            ));
            commands.push(SurfaceCommand::FillCircle {
                center: point,
                radius: playhead_glow_radius,
                color: theme.playhead_dot_glow,
            });
            commands.push(SurfaceCommand::FillCircle {
                center: point,
                radius: playhead_core_radius,
                color: theme.playhead_dot_core,
            });
            commands.push(SurfaceCommand::StrokeCircle {
                center: point,
                radius: playhead_core_radius.saturating_add(1),
                thickness: node_stroke.max(1),
                color: theme.playhead_dot_stroke,
            });
        }

        commands
    }

    pub(super) fn request_flush(&self) {
        if let Some(requester) = self.param_requester {
            requester.request_flush();
        }
    }

    /// Close any active knob automation gesture when pointer drag ends.
    pub(super) fn sync_knob_gesture_state(&self, mouse_down: bool, mouse_secondary_down: bool) {
        let mut ended_param = None;
        let mut finalize_marquee = false;
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.pointer_primary_down && !mouse_down {
                ended_param = runtime.active_knob_gesture_param.take();
                runtime.drag_mode = None;
                runtime.quick_slot_pressed = None;
                Self::commit_history_anchors_locked(&mut runtime);
            }
            if runtime.pointer_secondary_down
                && !mouse_secondary_down
                && runtime.marquee_selection.is_some()
            {
                finalize_marquee = true;
            }
            runtime.pointer_primary_down = mouse_down;
            runtime.pointer_secondary_down = mouse_secondary_down;
            if finalize_marquee {
                self.finalize_curve_marquee_selection_locked(&mut runtime);
            }
        }
        if let Some(param_id) = ended_param {
            self.end_param_gesture(param_id);
        }
    }

    pub(super) fn push_all_param_updates(&self) {
        self.push_single_value_update(PARAM_MIX_ID, self.params.mix() as f64);
        self.push_single_value_update(PARAM_PHASE_OFFSET_ID, self.params.phase_offset() as f64);
        self.push_single_value_update(PARAM_OUTPUT_GAIN_ID, self.params.output_gain_db() as f64);
        self.push_single_value_update(PARAM_SYNC_DIVISION_ID, self.params.sync_division() as f64);
    }

    pub(super) fn push_single_value_update(&self, param_id: ClapId, value: f64) {
        self.begin_param_gesture(param_id);
        self.push_param_value(param_id, value);
        self.end_param_gesture(param_id);
    }

    /// Push one knob update using drag-aware gesture boundaries.
    pub(super) fn push_knob_value_update(&self, param_id: ClapId, value: f64) {
        let mut should_begin = false;
        let mut should_end_previous = None;
        let mut immediate = false;
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.pointer_primary_down {
                if runtime.active_knob_gesture_param != Some(param_id) {
                    should_end_previous = runtime.active_knob_gesture_param.take();
                    runtime.active_knob_gesture_param = Some(param_id);
                    should_begin = true;
                }
            } else {
                should_end_previous = runtime.active_knob_gesture_param.take();
                immediate = true;
            }
        } else {
            immediate = true;
        }

        if let Some(previous) = should_end_previous {
            self.end_param_gesture(previous);
        }
        if immediate {
            self.push_single_value_update(param_id, value);
            return;
        }
        if should_begin {
            self.begin_param_gesture(param_id);
        }
        self.push_param_value(param_id, value);
    }

    /// Begin one automation gesture event.
    pub(super) fn begin_param_gesture(&self, param_id: ClapId) {
        let _status = self
            .automation_queue
            .push_gesture_begin(&self.automation_config, param_id);
        self.request_flush();
    }

    /// Push one automation value event.
    pub(super) fn push_param_value(&self, param_id: ClapId, value: f64) {
        let _status = self
            .automation_queue
            .push_value(&self.automation_config, param_id, value);
        self.request_flush();
    }

    /// End one automation gesture event.
    pub(super) fn end_param_gesture(&self, param_id: ClapId) {
        let _status = self
            .automation_queue
            .push_gesture_end(&self.automation_config, param_id);
        self.request_flush();
    }
}

pub(super) fn incoming_waveform_underlay_commands(
    size: Size,
    theme: PumpTheme,
    waveform: Option<crate::incoming_waveform::IncomingWaveformSnapshot>,
) -> Vec<SurfaceCommand> {
    let rect = Rect {
        origin: Point { x: 0, y: 0 },
        size,
    };
    let mut commands = Vec::with_capacity(
        crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT
            .saturating_mul(2)
            .saturating_add(1),
    );
    commands.push(SurfaceCommand::FillRect {
        rect,
        color: theme.curve_bg,
    });
    let Some(waveform) = waveform else {
        return commands;
    };

    let center_y = size.height.saturating_sub(1) as f32 * 0.5;
    let amplitude_scale = center_y * 0.86;
    let color = Color::rgba(
        theme.version_label.r,
        theme.version_label.g,
        theme.version_label.b,
        88,
    );
    let point = |index: usize, upper: bool| {
        let phase =
            index as f32 / (crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT - 1) as f32;
        let x = (phase * size.width.saturating_sub(1) as f32).round() as i32;
        let offset = waveform[index].clamp(0.0, 1.0) * amplitude_scale;
        Point {
            x,
            y: (center_y + if upper { -offset } else { offset }).round() as i32,
        }
    };
    for index in 1..crate::incoming_waveform::INCOMING_WAVEFORM_BIN_COUNT {
        commands.push(SurfaceCommand::Line {
            start: point(index - 1, true),
            end: point(index, true),
            color,
        });
        commands.push(SurfaceCommand::Line {
            start: point(index - 1, false),
            end: point(index, false),
            color,
        });
    }
    commands
}

pub(super) fn gain_reduction_meter_commands(
    size: Size,
    theme: PumpTheme,
    reduction_db: f32,
) -> Vec<SurfaceCommand> {
    let stroke = METER_STROKE.max(1) as u32;
    let meter_width = (METER_WIDTH.max(1) as u32).min(size.width.max(1));
    let meter_height = size.height.max(1);
    let meter_x = size.width.saturating_sub(meter_width) / 2;
    let meter_rect = Rect {
        origin: Point {
            x: meter_x as i32,
            y: 0,
        },
        size: Size {
            width: meter_width,
            height: meter_height,
        },
    };
    let mut commands = vec![SurfaceCommand::StrokeRect {
        rect: meter_rect,
        thickness: stroke,
        color: theme.meter_outline,
    }];
    let inner_width = meter_width.saturating_sub(stroke.saturating_mul(2));
    let inner_height = meter_height.saturating_sub(stroke.saturating_mul(2));
    let fill_height = ((inner_height as f32)
        * crate::gui_status::gain_reduction_meter_fraction(reduction_db))
    .round() as u32;
    if fill_height > 0 && inner_width > 0 {
        commands.push(SurfaceCommand::FillRect {
            rect: Rect {
                origin: Point {
                    x: meter_rect.origin.x + stroke as i32,
                    y: meter_rect.origin.y + stroke as i32,
                },
                size: Size {
                    width: inner_width,
                    height: fill_height,
                },
            },
            color: theme.meter_fill,
        });
    }
    for step in 1..3 {
        let y = ((meter_height.saturating_sub(1) * step) / 3) as i32;
        commands.push(SurfaceCommand::Line {
            start: Point {
                x: meter_rect.origin.x.saturating_sub(2),
                y,
            },
            end: Point {
                x: meter_rect.origin.x + meter_width as i32 + 1,
                y,
            },
            color: theme.meter_outline,
        });
    }
    commands
}

pub(super) fn curve_beat_grid_commands(
    size: Size,
    theme: PumpTheme,
    sync_division: usize,
) -> Vec<SurfaceCommand> {
    let max_x = size.width.saturating_sub(1) as f32;
    let max_y = size.height.saturating_sub(1) as i32;
    let grid = curve_beat_grid(sync_division, size.width as f32);
    let mut commands = Vec::with_capacity(grid.minor.len() + grid.major.len());
    for (positions, color) in [
        (grid.minor.as_slice(), theme.curve_grid_vertical),
        (grid.major.as_slice(), theme.curve_grid_emphasis),
    ] {
        for position in positions {
            let x = (position * max_x).round() as i32;
            commands.push(SurfaceCommand::Line {
                start: Point { x, y: 0 },
                end: Point { x, y: max_y },
                color,
            });
        }
    }
    commands
}

pub(super) fn curve_gain_reference_line_commands(
    size: Size,
    theme: PumpTheme,
) -> Vec<SurfaceCommand> {
    let max_x = size.width.saturating_sub(1) as i32;
    curve_gain_references()
        .into_iter()
        .map(|reference| {
            let y = local_from_node_for_size(
                CurveNode {
                    x: 0.0,
                    y: reference.gain,
                },
                size,
            )
            .y;
            SurfaceCommand::Line {
                start: Point { x: 0, y },
                end: Point { x: max_x, y },
                color: theme.curve_reference_line,
            }
        })
        .collect()
}

pub(super) fn curve_gain_reference_label_commands(
    size: Size,
    theme: PumpTheme,
    text_scale: u32,
) -> Vec<SurfaceCommand> {
    let text_scale = text_scale.max(1);
    let glyph_advance = 6_u32.saturating_mul(text_scale);
    let glyph_height = 7_u32.saturating_mul(text_scale);
    let label_right_padding = 4_u32;
    let label_vertical_padding = 2_u32;
    let mut commands = Vec::with_capacity(5);
    commands.push(SurfaceCommand::FillRect {
        rect: Rect {
            origin: Point { x: 0, y: 0 },
            size,
        },
        color: theme.curve_bg,
    });

    for reference in curve_gain_references() {
        let y = local_from_node_for_size(
            CurveNode {
                x: 0.0,
                y: reference.gain,
            },
            size,
        )
        .y;
        let text_width =
            (reference.bitmap_label.chars().count() as u32).saturating_mul(glyph_advance);
        let plate_height = glyph_height
            .saturating_add(label_vertical_padding.saturating_mul(2))
            .min(size.height);
        let plate_top =
            (y - plate_height as i32 / 2).clamp(0, size.height.saturating_sub(plate_height) as i32);
        let text_x = size
            .width
            .saturating_sub(text_width)
            .saturating_sub(label_right_padding) as i32;
        commands.push(SurfaceCommand::Text {
            origin: Point {
                x: text_x,
                y: plate_top.saturating_add(label_vertical_padding as i32),
            },
            text: reference.bitmap_label.to_string(),
            color: theme.curve_reference_label,
            scale: text_scale,
        });
    }

    commands
}
