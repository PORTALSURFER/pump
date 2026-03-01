use super::*;
impl GuiRuntime {
    pub(super) fn new() -> Self {
        Self {
            selected_node: None,
            drag_mode: None,
            curve_hovered: false,
            curve_local_pointer: Point { x: 0, y: 0 },
            curve_size: Size {
                width: CURVE_W,
                height: CURVE_H,
            },
            last_division_change_micros: None,
            preset_rename_active: false,
            preset_rename_target: 0,
            preset_name_draft: String::new(),
            preset_warning_frames: 0,
            preset_warning_text: None,
            pointer_primary_down: false,
            active_knob_gesture_param: None,
            undo_history: Vec::new(),
            redo_history: Vec::new(),
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
        ControlSnapshot {
            mix: self.params.mix(),
            depth: self.params.depth(),
            phase_offset: self.params.phase_offset(),
            output_gain_db: self.params.output_gain_db(),
            division: self.params.sync_division(),
        }
    }

    /// Snapshot preset-bank state and transient header interaction flags.
    pub(super) fn snapshot_presets(&self) -> PresetSnapshot {
        let bank = self.params.preset_bank_snapshot();
        let names = if bank.presets.is_empty() {
            vec![DEFAULT_PRESET_NAME.to_string()]
        } else {
            bank.presets
                .iter()
                .map(|preset| preset.name.clone())
                .collect()
        };
        let selected = bank.selected.min(names.len().saturating_sub(1));
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
            warning_blink_visible,
        }
    }

    pub(super) fn mark_division_change(&self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.last_division_change_micros = Some(monotonic_micros());
        }
    }

    pub(super) fn set_preset_warning(&self, text: &'static str) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.preset_warning_text = Some(text);
            runtime.preset_warning_frames = PRESET_WARNING_FRAMES;
        }
    }

    pub(super) fn consume_recent_division_change_guard(&self) -> bool {
        let Ok(mut runtime) = self.runtime.lock() else {
            return false;
        };
        let Some(last_division_change_micros) = runtime.last_division_change_micros else {
            return false;
        };
        let elapsed = monotonic_micros().saturating_sub(last_division_change_micros);
        if elapsed <= RESET_GUARD_AFTER_DROPDOWN_MICROS {
            // Swallow one immediate reset press after dropdown selection to avoid
            // accidental click-through while popup selection closes.
            runtime.last_division_change_micros = None;
            return true;
        }
        false
    }

    /// Build the top header slot node.
    pub(super) fn build_header_slot(
        &self,
        metrics: UiLayoutMetrics,
        theme: PumpTheme,
        presets: &PresetSnapshot,
    ) -> Node {
        let header_h = resolve_vertical_slot_heights(metrics.content_h).0.max(1);
        let header_slot_widths = weighted_slot_lengths(
            metrics.content_w.max(1),
            &[
                HEADER_EMPTY_SECTION_PERCENT as u16,
                HEADER_INDICATOR_SECTION_PERCENT as u16,
            ],
        );
        let left_width = header_slot_widths.first().copied().unwrap_or(1).max(1);
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
            weighted_slot(left_controls, HEADER_EMPTY_SECTION_PERCENT as u16),
            weighted_slot(indicator_node, HEADER_INDICATOR_SECTION_PERCENT as u16),
        ])
        .container_overflow(OverflowPolicy::Compress);
        panel("header", header_content).pad_all(0)
    }

    /// Build the spline/curve slot node.
    pub(super) fn build_spline_slot(&self, metrics: UiLayoutMetrics, theme: PumpTheme) -> Node {
        let editable_curve = self.params.editable_curve_snapshot();
        let curve_editor_view = curve_editor(CURVE_KEY, curve_model_from_editable(&editable_curve))
            .curve_style(curve_editor_style(theme))
            .curve_interaction(curve_editor_interaction_options(metrics.curve_size))
            .curve_playhead_x(
                (self.status.has_host_beats_timeline() || self.status.is_playing())
                    .then_some(self.status.phase()),
            )
            .widget_layout(fixed_box(metrics.content_w, metrics.curve_size.height))
            .fill();
        let spline_content = Node::padding_box(curve_editor_view)
            .pad_xy(0, CURVE_VERTICAL_MARGIN as i32)
            .widget_layout(fixed_box(metrics.content_w, metrics.curve_h))
            .fill();
        panel("spline", spline_content).pad_all(0)
    }

    /// Build the controls slot node.
    pub(super) fn build_controls_slot(
        &self,
        metrics: UiLayoutMetrics,
        controls: ControlSnapshot,
    ) -> Node {
        const KNOB_TEXT_MAX_CHARS: u32 = 8;
        const MONO_CHAR_CELL_WIDTH_PX: u32 = 6;
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
                    DEPTH_KEY,
                    "Depth",
                    controls.depth,
                    DEFAULT_DEPTH,
                    (MIN_DEPTH, MAX_DEPTH),
                    format!("{:.0}%", controls.depth * 100.0),
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
            button(RESET_KEY)
                .button_label("Reset")
                .control_size(Size {
                    width: metrics.dropdown_control_w,
                    height: metrics.button_control_h,
                })
                .fill(),
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
        self.sync_knob_gesture_state(input.mouse_down);
        let metrics = UiLayoutMetrics::design_space();
        let theme = PumpTheme::main(metrics);
        let controls = self.snapshot_controls();
        let presets = self.snapshot_presets();

        let header_slot = self.build_header_slot(metrics, theme, &presets);
        let spline_slot = self.build_spline_slot(metrics, theme);
        let controls_slot = self.build_controls_slot(metrics, controls);

        let content = column_slots(vec![
            weighted_slot(header_slot, HEADER_SECTION_WEIGHT),
            weighted_slot(spline_slot, CURVE_SECTION_WEIGHT),
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
                if matches!(key.as_str(), MIX_KEY | DEPTH_KEY | PHASE_KEY | OUTPUT_KEY) {
                    self.capture_undo_snapshot();
                }
                self.reduce_knob(key.as_str(), value);
            }
            UiAction::DropdownSelected { key, index } => {
                if matches!(key.as_str(), PRESET_DROPDOWN_KEY | DIVISION_KEY) {
                    self.capture_undo_snapshot();
                }
                self.reduce_dropdown(key.as_str(), index);
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
            UiAction::ButtonPressed { key } if key == RESET_KEY => {
                if self.consume_recent_division_change_guard() {
                    return;
                }
                self.capture_undo_snapshot();
                self.params.reset_curve_to_default();
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.selected_node = None;
                    runtime.drag_mode = None;
                }
            }
            UiAction::ButtonPressed { key } if key == PRESET_ADD_KEY => {
                self.capture_undo_snapshot();
                if self.params.add_preset_from_current_state().is_some() {
                    if let Ok(mut runtime) = self.runtime.lock() {
                        runtime.preset_rename_active = false;
                        runtime.preset_name_draft.clear();
                        runtime.preset_warning_frames = 0;
                        runtime.preset_warning_text = None;
                    }
                } else {
                    self.set_preset_warning(PRESET_WARNING_MAX);
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
                }
            }
            UiAction::RegionHover { .. } => {}
            UiAction::RegionInteracted {
                key,
                kind,
                local_pointer,
                raw_local_pointer,
                alt_down,
            } if key == CURVE_KEY => {
                self.reduce_curve_interaction(kind, local_pointer, raw_local_pointer, alt_down)
            }
            UiAction::RegionInteracted { .. } => {}
            _ => {}
        }
    }

    fn snapshot_history_state(&self) -> UiHistorySnapshot {
        UiHistorySnapshot {
            mix: self.params.mix(),
            depth: self.params.depth(),
            phase_offset: self.params.phase_offset(),
            output_gain_db: self.params.output_gain_db(),
            sync_division: self.params.sync_division(),
            editable_curve: self.params.editable_curve_snapshot(),
            preset_bank: self.params.preset_bank_snapshot(),
        }
    }

    fn apply_history_state(&self, snapshot: &UiHistorySnapshot) {
        self.params.set_preset_bank(snapshot.preset_bank.clone());
        self.params.set_mix(snapshot.mix);
        self.params.set_depth(snapshot.depth);
        self.params.set_phase_offset(snapshot.phase_offset);
        self.params.set_output_gain_db(snapshot.output_gain_db);
        self.params.set_sync_division(snapshot.sync_division as f32);
        self.params.set_editable_curve(&snapshot.editable_curve);
        self.push_all_param_updates();
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

    fn capture_undo_snapshot(&self) {
        let snapshot = self.snapshot_history_state();
        if let Ok(mut runtime) = self.runtime.lock() {
            Self::commit_curve_history_anchor_locked(&mut runtime);
            Self::push_undo_snapshot_locked(&mut runtime, snapshot);
            runtime.curve_history_anchor = None;
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

    fn apply_undo(&self) {
        let current = self.snapshot_history_state();
        let mut target = None;
        if let Ok(mut runtime) = self.runtime.lock() {
            if let Some(snapshot) = runtime.undo_history.pop() {
                Self::push_history_snapshot(&mut runtime.redo_history, current);
                runtime.curve_history_anchor = None;
                runtime.drag_mode = None;
                runtime.selected_node = None;
                runtime.preset_rename_active = false;
                runtime.preset_name_draft.clear();
                runtime.preset_warning_frames = 0;
                runtime.preset_warning_text = None;
                target = Some(snapshot);
            }
        }
        if let Some(snapshot) = target {
            self.apply_history_state(&snapshot);
        }
    }

    fn apply_redo(&self) {
        let current = self.snapshot_history_state();
        let mut target = None;
        if let Ok(mut runtime) = self.runtime.lock() {
            if let Some(snapshot) = runtime.redo_history.pop() {
                Self::push_history_snapshot(&mut runtime.undo_history, current);
                runtime.curve_history_anchor = None;
                runtime.drag_mode = None;
                runtime.selected_node = None;
                runtime.preset_rename_active = false;
                runtime.preset_name_draft.clear();
                runtime.preset_warning_frames = 0;
                runtime.preset_warning_text = None;
                target = Some(snapshot);
            }
        }
        if let Some(snapshot) = target {
            self.apply_history_state(&snapshot);
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
            DEPTH_KEY => {
                self.params.set_depth(value);
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
            if let Some(selected) = self.params.load_preset(index) {
                self.push_all_param_updates();
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.preset_rename_active = false;
                    runtime.preset_name_draft.clear();
                    runtime.preset_rename_target = selected;
                    runtime.preset_warning_frames = 0;
                    runtime.preset_warning_text = None;
                }
            }
            return;
        }

        if key != DIVISION_KEY {
            return;
        }

        let clamped = index.min(MAX_SYNC_DIVISION as usize);
        self.mark_division_change();
        self.params.set_sync_division(clamped as f32);
        self.push_single_value_update(PARAM_SYNC_DIVISION_ID, clamped as f64);
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
        if renamed {
            runtime.preset_warning_frames = 0;
            runtime.preset_warning_text = None;
        } else {
            runtime.preset_warning_text = Some(PRESET_WARNING_NAME);
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
            SavePresetOutcome::Overwritten { index } | SavePresetOutcome::Created { index } => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.preset_rename_active = false;
                    runtime.preset_name_draft.clear();
                    runtime.preset_rename_target = index;
                    runtime.preset_warning_frames = 0;
                    runtime.preset_warning_text = None;
                }
            }
            SavePresetOutcome::BlockedFull => self.set_preset_warning(PRESET_WARNING_MAX),
            SavePresetOutcome::InvalidName => self.set_preset_warning(PRESET_WARNING_NAME),
        }
    }

    pub(super) fn reduce_curve_interaction(
        &mut self,
        kind: RegionInteractionKind,
        local_pointer: Point,
        raw_local_pointer: Point,
        alt_down: bool,
    ) {
        let Ok(mut runtime) = self.runtime.lock() else {
            return;
        };

        let local_pointer = scale_point_to_design(local_pointer, runtime.curve_size);
        let raw_local_pointer = scale_point_to_design(raw_local_pointer, runtime.curve_size);
        let normalized_pointer = node_from_local_for_size(local_pointer, runtime.curve_size);
        let raw_normalized_pointer =
            node_from_local_for_size(raw_local_pointer, runtime.curve_size);

        match kind {
            RegionInteractionKind::Pressed => {
                let mut editable = self.params.editable_curve_snapshot();
                if let Some(index) = find_node_hit_for_size(
                    &editable,
                    local_pointer,
                    node_hit_radius(runtime.curve_size),
                    runtime.curve_size,
                ) {
                    runtime.selected_node = Some(index);
                    runtime.drag_mode = Some(CurveDragMode::MoveNode {
                        origin_index: index,
                        origin_curve: editable.clone(),
                        start_pointer: local_pointer,
                        dragging: false,
                    });
                    return;
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
                    let preview_node = preview_node_on_curve_for_size(
                        &editable,
                        local_pointer,
                        runtime.curve_size,
                    )
                    .unwrap_or(normalized_pointer);
                    let inserted_index =
                        insert_node_for_size(&mut editable, preview_node, runtime.curve_size);
                    runtime.selected_node = Some(inserted_index);
                    runtime.drag_mode = Some(CurveDragMode::MoveNode {
                        origin_index: inserted_index,
                        origin_curve: editable.clone(),
                        start_pointer: local_pointer,
                        dragging: false,
                    });
                    Self::push_undo_snapshot_locked(&mut runtime, self.snapshot_history_state());
                    enforce_wrapped_endpoints(&mut editable);
                    self.params.set_editable_curve(&editable);
                    return;
                }

                if let Some(index) = find_segment_line_hit_within_for_size(
                    &editable,
                    local_pointer,
                    segment_near_hit_radius(runtime.curve_size),
                    runtime.curve_size,
                ) {
                    runtime.drag_mode = if alt_down {
                        let start_tension = editable
                            .segments
                            .get(index)
                            .copied()
                            .unwrap_or(CurveSegment { tension: 0.0 })
                            .tension;
                        Some(CurveDragMode::AdjustSegmentCurve {
                            index,
                            start_pointer: local_pointer,
                            start_tension,
                            dragging: false,
                        })
                    } else {
                        let right_index = (index + 1).min(editable.nodes.len().saturating_sub(1));
                        Some(CurveDragMode::MoveSegment {
                            index,
                            start_pointer: local_pointer,
                            start_left_x: editable.nodes[index].x,
                            start_right_x: editable.nodes[right_index].x,
                            start_left_y: editable.nodes[index].y,
                            start_right_y: editable.nodes[right_index].y,
                            dragging: false,
                        })
                    };
                    return;
                }

                if let Some(index) = find_node_hit_within_for_size(
                    &editable,
                    local_pointer,
                    node_insert_guard_radius(runtime.curve_size),
                    runtime.curve_size,
                ) {
                    runtime.selected_node = Some(index);
                    runtime.drag_mode = Some(CurveDragMode::MoveNode {
                        origin_index: index,
                        origin_curve: editable.clone(),
                        start_pointer: local_pointer,
                        dragging: false,
                    });
                    return;
                }

                let inserted_index =
                    insert_node_for_size(&mut editable, normalized_pointer, runtime.curve_size);
                runtime.selected_node = Some(inserted_index);
                runtime.drag_mode = Some(CurveDragMode::MoveNode {
                    origin_index: inserted_index,
                    origin_curve: editable.clone(),
                    start_pointer: local_pointer,
                    dragging: false,
                });
                Self::push_undo_snapshot_locked(&mut runtime, self.snapshot_history_state());
                enforce_wrapped_endpoints(&mut editable);
                self.params.set_editable_curve(&editable);
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
                            let (recomputed_curve, moved_index) =
                                recompute_move_node_from_origin_for_size(
                                    &origin_curve,
                                    origin_index,
                                    raw_normalized_pointer,
                                    node_push_through_threshold_px(runtime.curve_size),
                                    runtime.curve_size,
                                );
                            editable = recomputed_curve;
                            runtime.selected_node = Some(moved_index);
                            drag_mode = CurveDragMode::MoveNode {
                                origin_index,
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
                    runtime.drag_mode = None;
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
        let meter_x_offset = metrics.meter_x_offset.max(0);
        let meter_y_offset = metrics.meter_y_offset.max(0);
        let meter_width = metrics.meter_width.max(1);
        let meter_width_i32 = i32::try_from(meter_width).unwrap_or(i32::MAX);
        let meter_stroke_u32 = metrics.meter_stroke.max(1);
        let meter_inner_width = metrics
            .meter_width
            .max(1)
            .saturating_sub(meter_stroke_u32.saturating_mul(2));

        let mut commands = Vec::with_capacity(1024);
        commands.push(SurfaceCommand::FillRect {
            rect,
            color: theme.curve_bg,
        });
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

        for step in 1..4 {
            let y = ((curve_size.height as i32 - 1) * step) / 4;
            commands.push(SurfaceCommand::Line {
                start: Point { x: 0, y },
                end: Point {
                    x: curve_size.width as i32 - 1,
                    y,
                },
                color: theme.curve_grid_horizontal,
            });
        }

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
            let selected = state.selected_node == Some(index);
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

        let reduction = (1.0 - self.status.gain().clamp(0.0, 1.0)).clamp(0.0, 1.0);
        let meter_rect = Rect {
            origin: Point {
                x: curve_size.width as i32 - meter_x_offset - meter_width_i32,
                y: meter_y_offset,
            },
            size: Size {
                width: meter_width,
                height: curve_size
                    .height
                    .saturating_sub((meter_y_offset.saturating_mul(2)).max(0) as u32),
            },
        };
        commands.push(SurfaceCommand::StrokeRect {
            rect: meter_rect,
            thickness: meter_stroke_u32,
            color: theme.meter_outline,
        });
        let fill_height = ((meter_rect.size.height as f32) * reduction).round() as u32;
        if fill_height > 0 {
            commands.push(SurfaceCommand::FillRect {
                rect: Rect {
                    origin: Point {
                        x: meter_rect.origin.x
                            + i32::try_from(meter_stroke_u32).unwrap_or(i32::MAX),
                        y: meter_rect.origin.y
                            + i32::try_from(meter_stroke_u32).unwrap_or(i32::MAX),
                    },
                    size: Size {
                        width: meter_inner_width,
                        height: fill_height,
                    },
                },
                color: theme.meter_fill,
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
    pub(super) fn sync_knob_gesture_state(&self, mouse_down: bool) {
        let mut ended_param = None;
        if let Ok(mut runtime) = self.runtime.lock() {
            if runtime.pointer_primary_down && !mouse_down {
                ended_param = runtime.active_knob_gesture_param.take();
                Self::commit_curve_history_anchor_locked(&mut runtime);
            }
            runtime.pointer_primary_down = mouse_down;
        }
        if let Some(param_id) = ended_param {
            self.end_param_gesture(param_id);
        }
    }

    pub(super) fn push_all_param_updates(&self) {
        self.push_single_value_update(PARAM_MIX_ID, self.params.mix() as f64);
        self.push_single_value_update(PARAM_DEPTH_ID, self.params.depth() as f64);
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
