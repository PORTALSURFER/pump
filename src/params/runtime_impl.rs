use super::*;
impl PumpParams {
    #[inline]
    fn realtime_index(&self) -> usize {
        (self.active_sound.load(Ordering::Acquire) as usize).min(1)
    }

    /// Create params with production defaults and default curve.
    pub fn new() -> Self {
        let editable_curve = default_editable_curve();
        let default_curve = editable_curve_to_table(&editable_curve);
        let params = Self {
            mix: AtomicF32::new(DEFAULT_MIX),
            depth_db: AtomicF32::new(DEFAULT_DEPTH_DB),
            floor_db: AtomicF32::new(DEFAULT_FLOOR_DB),
            phase_offset: AtomicF32::new(DEFAULT_PHASE_OFFSET),
            output_gain_db: AtomicF32::new(DEFAULT_OUTPUT_GAIN_DB),
            sync_division: AtomicU32::new(DEFAULT_SYNC_DIVISION_INDEX as u32),
            trigger_mode: AtomicU32::new(DEFAULT_TRIGGER_MODE as u32),
            smooth: AtomicF32::new(DEFAULT_SMOOTH),
            mode: AtomicU32::new(PROCESSING_MODE_CLASSIC as u32),
            swing: AtomicF32::new(DEFAULT_SWING),
            bypass: AtomicBool::new(false),
            bypass_revision: AtomicU32::new(1),
            bypass_last_automation_micros: AtomicU64::new(0),
            editable_curve: RwLock::new(editable_curve),
            curve: std::array::from_fn(|index| AtomicF32::new(default_curve[index])),
            curve_revision: AtomicU32::new(1),
            preset_bank: RwLock::new(PumpPresetBank::default_init()),
            preset_persistence_warning: RwLock::new(None),
            active_sound: AtomicU32::new(SoundSide::A.index() as u32),
            pending_active_sound: AtomicU32::new(u32::MAX),
            active_sound_dirty: AtomicBool::new(false),
            realtime_mix: std::array::from_fn(|_| AtomicF32::new(DEFAULT_MIX)),
            realtime_depth_db: std::array::from_fn(|_| AtomicF32::new(DEFAULT_DEPTH_DB)),
            realtime_floor_db: std::array::from_fn(|_| AtomicF32::new(DEFAULT_FLOOR_DB)),
            realtime_phase_offset: std::array::from_fn(|_| AtomicF32::new(DEFAULT_PHASE_OFFSET)),
            realtime_output_gain_db: std::array::from_fn(|_| {
                AtomicF32::new(DEFAULT_OUTPUT_GAIN_DB)
            }),
            realtime_sync_division: std::array::from_fn(|_| {
                AtomicU32::new(DEFAULT_SYNC_DIVISION_INDEX as u32)
            }),
            realtime_trigger_mode: std::array::from_fn(|_| {
                AtomicU32::new(DEFAULT_TRIGGER_MODE as u32)
            }),
            realtime_smooth: std::array::from_fn(|_| AtomicF32::new(DEFAULT_SMOOTH)),
            realtime_mode: std::array::from_fn(|_| AtomicU32::new(PROCESSING_MODE_CLASSIC as u32)),
            realtime_swing: std::array::from_fn(|_| AtomicF32::new(DEFAULT_SWING)),
            realtime_curve: std::array::from_fn(|_| {
                std::array::from_fn(|index| AtomicF32::new(default_curve[index]))
            }),
            sound_states: RwLock::new([
                PumpSoundState::default_init(),
                PumpSoundState::default_init(),
            ]),
        };
        match super::preset_store::load_persisted_preset_bank() {
            Ok(Some(bank)) => params.set_preset_bank_without_persistence(bank),
            Ok(None) => {}
            Err(error) => {
                params.record_preset_persistence_failure("load", error.as_str());
            }
        }
        params
    }

    /// Get dry/wet mix amount.
    pub fn mix(&self) -> f32 {
        self.realtime_mix[self.realtime_index()].load(Ordering::Relaxed)
    }

    /// Get curve attenuation depth in decibels.
    pub fn depth_db(&self) -> f32 {
        self.realtime_depth_db[self.realtime_index()].load(Ordering::Relaxed)
    }

    /// Get the minimum wet gain floor in decibels.
    pub fn floor_db(&self) -> f32 {
        self.realtime_floor_db[self.realtime_index()].load(Ordering::Relaxed)
    }

    /// Get legacy normalized depth amount for old payloads and presets.
    pub fn depth(&self) -> f32 {
        (self.depth_db() / MAX_DEPTH_DB).clamp(0.0, 1.0)
    }

    /// Get cycle phase offset.
    pub fn phase_offset(&self) -> f32 {
        self.realtime_phase_offset[self.realtime_index()].load(Ordering::Relaxed)
    }

    /// Get output gain in decibels.
    pub fn output_gain_db(&self) -> f32 {
        self.realtime_output_gain_db[self.realtime_index()].load(Ordering::Relaxed)
    }

    /// Get sync division index.
    pub fn sync_division(&self) -> usize {
        clamp_sync_division(
            self.realtime_sync_division[self.realtime_index()].load(Ordering::Relaxed) as f32,
        )
    }

    /// Get sync division in beats per cycle.
    pub fn sync_beats_per_cycle(&self) -> f32 {
        sync_division_beats(self.sync_division())
    }

    /// Get the selected curve trigger source.
    pub fn trigger_mode(&self) -> usize {
        clamp_trigger_mode(
            self.realtime_trigger_mode[self.realtime_index()].load(Ordering::Relaxed) as f32,
        )
    }

    /// Get evaluated wet-gain smoothing amount.
    pub fn smooth(&self) -> f32 {
        self.realtime_smooth[self.realtime_index()].load(Ordering::Relaxed)
    }

    /// Get the sole supported processing mode, mapping historical values to Classic.
    pub fn mode(&self) -> usize {
        clamp_processing_mode(
            self.realtime_mode[self.realtime_index()].load(Ordering::Relaxed) as f32,
        )
    }

    /// Get alternating-subdivision swing amount.
    pub fn swing(&self) -> f32 {
        self.realtime_swing[self.realtime_index()]
            .load(Ordering::Relaxed)
            .clamp(MIN_SWING, MAX_SWING)
    }

    /// Return whether complete Pump output is currently bypassed.
    pub fn bypassed(&self) -> bool {
        self.bypass.load(Ordering::Relaxed)
    }

    /// Return the bypass value in its plain host representation.
    pub fn bypass_value(&self) -> f32 {
        if self.bypassed() {
            BYPASS_BYPASSED_VALUE
        } else {
            BYPASS_ACTIVE_VALUE
        }
    }

    /// Read the bypass projection revision used by hosted editors.
    pub fn bypass_revision(&self) -> u32 {
        self.bypass_revision.load(Ordering::Acquire)
    }

    /// Return whether host bypass automation was observed in the last 250 ms.
    pub fn bypass_automation_recent(&self) -> bool {
        let last = self.bypass_last_automation_micros.load(Ordering::Acquire);
        last != 0
            && crate::time_utils::monotonic_micros().saturating_sub(last)
                < BYPASS_AUTOMATION_CUE_MICROS
    }

    /// Set dry/wet mix amount.
    pub fn set_mix(&self, value: f32) {
        let value = value.clamp(MIN_MIX, MAX_MIX);
        self.mix.store(value, Ordering::Relaxed);
        self.realtime_mix[self.realtime_index()].store(value, Ordering::Relaxed);
        self.mark_active_sound_dirty();
    }

    /// Set curve attenuation depth in decibels.
    pub fn set_depth_db(&self, value: f32) {
        let value = if value.is_finite() {
            value
        } else {
            DEFAULT_DEPTH_DB
        };
        let value = value.clamp(MIN_DEPTH_DB, MAX_DEPTH_DB);
        self.depth_db.store(value, Ordering::Relaxed);
        self.realtime_depth_db[self.realtime_index()].store(value, Ordering::Relaxed);
        self.mark_active_sound_dirty();
    }

    /// Set legacy normalized depth, migrating it to the documented dB range.
    pub fn set_depth(&self, value: f32) {
        self.set_depth_db(value.clamp(0.0, 1.0) * MAX_DEPTH_DB);
    }

    /// Set the minimum wet gain floor in decibels. The minimum value is −∞.
    pub fn set_floor_db(&self, value: f32) {
        let value = if value.is_finite() {
            value
        } else {
            DEFAULT_FLOOR_DB
        };
        let value = value.clamp(MIN_FLOOR_DB, MAX_FLOOR_DB);
        self.floor_db.store(value, Ordering::Relaxed);
        self.realtime_floor_db[self.realtime_index()].store(value, Ordering::Relaxed);
        self.mark_active_sound_dirty();
    }

    /// Set cycle phase offset.
    pub fn set_phase_offset(&self, value: f32) {
        let value = value.clamp(MIN_PHASE_OFFSET, MAX_PHASE_OFFSET);
        self.phase_offset.store(value, Ordering::Relaxed);
        self.realtime_phase_offset[self.realtime_index()].store(value, Ordering::Relaxed);
        self.mark_active_sound_dirty();
    }

    /// Set output gain in decibels.
    pub fn set_output_gain_db(&self, value: f32) {
        let value = value.clamp(MIN_OUTPUT_GAIN_DB, MAX_OUTPUT_GAIN_DB);
        self.output_gain_db.store(value, Ordering::Relaxed);
        self.realtime_output_gain_db[self.realtime_index()].store(value, Ordering::Relaxed);
        self.mark_active_sound_dirty();
    }

    /// Set sync division index from scalar host value.
    pub fn set_sync_division(&self, value: f32) {
        let index = clamp_sync_division(value);
        self.sync_division.store(index as u32, Ordering::Relaxed);
        self.realtime_sync_division[self.realtime_index()].store(index as u32, Ordering::Relaxed);
        self.mark_active_sound_dirty();
    }

    /// Set the curve trigger source from a scalar host value.
    pub fn set_trigger_mode(&self, value: f32) {
        let value = clamp_trigger_mode(value) as u32;
        self.trigger_mode.store(value, Ordering::Relaxed);
        self.realtime_trigger_mode[self.realtime_index()].store(value, Ordering::Relaxed);
        self.mark_active_sound_dirty();
    }

    /// Set evaluated wet-gain smoothing amount.
    pub fn set_smooth(&self, value: f32) {
        let value = if value.is_finite() {
            value
        } else {
            DEFAULT_SMOOTH
        };
        let value = value.clamp(MIN_SMOOTH, MAX_SMOOTH);
        self.smooth.store(value, Ordering::Relaxed);
        self.realtime_smooth[self.realtime_index()].store(value, Ordering::Relaxed);
        self.mark_active_sound_dirty();
    }

    /// Set the processing mode, mapping historical values to Classic.
    pub fn set_mode(&self, value: f32) {
        let value = clamp_processing_mode(value) as u32;
        self.mode.store(value, Ordering::Relaxed);
        self.realtime_mode[self.realtime_index()].store(value, Ordering::Relaxed);
        self.mark_active_sound_dirty();
    }

    /// Set alternating-subdivision swing amount.
    pub fn set_swing(&self, value: f32) {
        let value = if value.is_finite() {
            value
        } else {
            DEFAULT_SWING
        };
        let value = value.clamp(MIN_SWING, MAX_SWING);
        self.swing.store(value, Ordering::Relaxed);
        self.realtime_swing[self.realtime_index()].store(value, Ordering::Relaxed);
        self.mark_active_sound_dirty();
    }

    /// Set bypass from its stepped plain host value.
    pub fn set_bypass(&self, value: f32) {
        let bypassed = value.is_finite() && value.round() >= BYPASS_BYPASSED_VALUE;
        if self.bypass.swap(bypassed, Ordering::AcqRel) != bypassed {
            self.bypass_revision.fetch_add(1, Ordering::Release);
        }
    }

    /// Set bypass from host automation and publish the GUI automation cue.
    pub(crate) fn set_bypass_from_host(&self, value: f32) {
        self.set_bypass(value);
        self.bypass_last_automation_micros.store(
            crate::time_utils::monotonic_micros().max(1),
            Ordering::Release,
        );
    }

    /// Read the current curve revision counter.
    pub fn curve_revision(&self) -> u32 {
        self.curve_revision.load(Ordering::Acquire)
    }

    /// Read one point from the curve table.
    pub fn curve_value(&self, index: usize) -> f32 {
        self.realtime_curve[self.realtime_index()]
            .get(index)
            .map(|sample: &AtomicF32| sample.load(Ordering::Acquire).clamp(0.0, 1.0))
            .unwrap_or(1.0)
    }

    /// Snapshot the whole curve table in one array.
    pub fn curve_snapshot(&self) -> [f32; CURVE_TABLE_LEN] {
        let mut values = [1.0_f32; CURVE_TABLE_LEN];
        let active_curve = &self.realtime_curve[self.realtime_index()];
        for (index, value) in values.iter_mut().enumerate() {
            *value = active_curve[index].load(Ordering::Acquire).clamp(0.0, 1.0);
        }
        values
    }

    /// Snapshot the editable spline curve.
    pub fn editable_curve_snapshot(&self) -> EditableCurve {
        self.editable_curve
            .read()
            .map(|curve| curve.clone())
            .unwrap_or_else(|_| default_editable_curve())
    }

    /// Replace the editable spline curve, regenerate the table, and advance revision.
    pub fn set_editable_curve(&self, editable_curve: &EditableCurve) {
        let mut plain_curve = editable_curve.clone();
        plain_curve.phase_source = None;
        plain_curve.phase_offset = 0.0;
        self.set_editable_curve_internal(&plain_curve);
        self.sync_active_sound_state();
    }

    /// Replace the editable curve while retaining an exact cyclic phase source.
    pub fn set_editable_curve_preserving_phase(&self, editable_curve: &EditableCurve) {
        self.set_editable_curve_internal(editable_curve);
        self.sync_active_sound_state();
    }

    fn set_editable_curve_internal(&self, editable_curve: &EditableCurve) {
        let normalized = editable_curve.clone().normalized();
        let curve_table = editable_curve_to_table(&normalized);
        if let Ok(mut guard) = self.editable_curve.write() {
            *guard = normalized;
        }
        self.store_curve_table(&curve_table);
        self.curve_revision.fetch_add(1, Ordering::AcqRel);
        self.mark_active_sound_dirty();
    }

    /// Replace the whole curve table and advance revision.
    pub fn set_curve(&self, values: &[f32; CURVE_TABLE_LEN]) {
        if let Ok(mut guard) = self.editable_curve.write() {
            *guard = curve_table_to_editable(values);
        }
        self.store_curve_table(values);
        self.curve_revision.fetch_add(1, Ordering::AcqRel);
        self.sync_active_sound_state();
    }

    /// Restore default curve shape and advance revision.
    pub fn reset_curve_to_default(&self) {
        self.set_editable_curve(&default_editable_curve());
    }

    /// Return the currently active A/B side.
    pub fn active_sound(&self) -> SoundSide {
        if self.active_sound.load(Ordering::Acquire) == SoundSide::B.index() as u32 {
            SoundSide::B
        } else {
            SoundSide::A
        }
    }

    /// Return a UI-safe snapshot of one complete A/B side.
    pub fn sound_state_snapshot(&self, side: SoundSide) -> PumpSoundState {
        if side == self.active_sound() && self.active_sound_dirty.load(Ordering::Acquire) {
            return self.current_audio_state();
        }
        self.sound_states
            .read()
            .ok()
            .and_then(|states| states.get(side.index()).cloned())
            .unwrap_or_else(PumpSoundState::default_init)
    }

    /// Return whether the two complete sound sides differ.
    pub fn sound_sides_differ(&self) -> bool {
        self.sync_active_sound_state();
        self.sound_states
            .read()
            .map(|states| states[0] != states[1])
            .unwrap_or(false)
    }

    /// Select one side and publish its complete state to the audio-facing atomics.
    ///
    /// The audio processor only reads atomics and never takes this lock. The
    /// returned value indicates whether a side transition occurred.
    pub fn set_active_sound(&self, side: SoundSide) -> bool {
        let current = self.active_sound();
        if current == side {
            return false;
        }
        self.sync_active_sound_state();
        let target = self
            .sound_states
            .read()
            .ok()
            .and_then(|states| states.get(side.index()).cloned())
            .unwrap_or_else(PumpSoundState::default_init);
        self.publish_sound_state_at(side.index(), &target);
        self.active_sound
            .store(side.index() as u32, Ordering::Release);
        self.curve_revision.fetch_add(1, Ordering::AcqRel);
        self.reconcile_active_sound_projection();
        true
    }

    /// Queue a host-driven side change without taking a lock on the audio thread.
    pub(crate) fn request_active_sound_from_host(&self, side: SoundSide) {
        // Every per-side audio field is pre-published in lock-free atomics;
        // changing this selector is therefore the complete realtime-safe
        // transition and requires no UI snapshot access.
        if self
            .active_sound
            .swap(side.index() as u32, Ordering::AcqRel)
            != side.index() as u32
        {
            self.curve_revision.fetch_add(1, Ordering::AcqRel);
        }
        self.pending_active_sound
            .store(side.index() as u32, Ordering::Release);
    }

    /// Apply a queued host side change from the UI/host projection thread.
    pub(crate) fn consume_pending_active_sound(&self) -> Option<SoundSide> {
        let value = self.pending_active_sound.swap(u32::MAX, Ordering::AcqRel);
        let side = if value == SoundSide::B.index() as u32 {
            SoundSide::B
        } else if value == SoundSide::A.index() as u32 {
            SoundSide::A
        } else {
            return None;
        };
        if self.active_sound_dirty.load(Ordering::Acquire) {
            self.sync_sound_state_from_realtime(SoundSide::A);
            self.sync_sound_state_from_realtime(SoundSide::B);
        }
        self.reconcile_active_sound_projection();
        Some(side)
    }

    pub(crate) fn has_pending_active_sound(&self) -> bool {
        self.pending_active_sound.load(Ordering::Acquire) != u32::MAX
    }

    /// Copy the active complete sound state into the inactive side.
    ///
    /// Copying intentionally does not republish the active atomics, so the
    /// currently audible state remains unchanged.
    pub fn copy_active_to_inactive(&self) -> bool {
        let active = self.active_sound();
        self.sync_active_sound_state();
        let snapshot = self.current_audio_state();
        let Ok(mut states) = self.sound_states.write() else {
            return false;
        };
        let inactive = active.other();
        if states[inactive.index()] == snapshot {
            return false;
        }
        self.publish_sound_state_at(inactive.index(), &snapshot);
        states[inactive.index()] = snapshot;
        true
    }

    /// Set one active side's quick slots without changing its audio state.
    pub fn set_active_sound_quick_slots(
        &self,
        quick_slots: Vec<QuickShapeSlot>,
    ) -> Result<(), PresetMutationError> {
        let mut quick_slots = quick_slots;
        quick_slots.truncate(QUICK_SLOT_COUNT);
        if quick_slots.len() != QUICK_SLOT_COUNT {
            return Err(PresetMutationError::InvalidIndex);
        }
        let Ok(mut states) = self.sound_states.write() else {
            return Err(PresetMutationError::StateUnavailable);
        };
        states[self.active_sound().index()].quick_slots = quick_slots;
        Ok(())
    }

    fn current_audio_state(&self) -> PumpSoundState {
        // The editable curve projection is UI-owned and may still represent
        // the previously selected side when host automation switches sides
        // with the editor closed. Keep the stored side curve as the source of
        // truth for scalar-only dirty snapshots; UI curve edits synchronize
        // this stored value immediately through `sync_active_sound_state`.
        let editable_curve = self
            .sound_states
            .read()
            .ok()
            .and_then(|states| {
                states
                    .get(self.active_sound().index())
                    .map(|state| state.editable_curve.clone())
            })
            .unwrap_or_else(default_editable_curve);
        let quick_slots = self
            .sound_states
            .read()
            .ok()
            .and_then(|states| {
                states
                    .get(self.active_sound().index())
                    .map(|state| state.quick_slots.clone())
            })
            .unwrap_or_else(seeded_quick_shape_slots);
        PumpSoundState {
            mix: self.mix(),
            depth_db: self.depth_db(),
            floor_db: self.floor_db(),
            phase_offset: self.phase_offset(),
            output_gain_db: self.output_gain_db(),
            sync_division: self.sync_division(),
            trigger_mode: self.trigger_mode(),
            smooth: self.smooth(),
            mode: self.mode(),
            swing: self.swing(),
            editable_curve,
            quick_slots,
        }
    }

    fn sync_active_sound_state(&self) {
        let snapshot = self.current_audio_state();
        // Public curve mutators update the UI projection before reaching this
        // helper, so capture that exact editable representation rather than
        // reducing it back through the stored snapshot curve.
        let snapshot = PumpSoundState {
            editable_curve: self.editable_curve_snapshot(),
            ..snapshot
        };
        if let Ok(mut states) = self.sound_states.write() {
            states[self.active_sound().index()] = snapshot;
            self.active_sound_dirty.store(false, Ordering::Release);
        }
    }

    fn sync_sound_state_from_realtime(&self, side: SoundSide) {
        let index = side.index();
        let quick_slots = self
            .sound_states
            .read()
            .ok()
            .and_then(|states| states.get(index).map(|state| state.quick_slots.clone()))
            .unwrap_or_else(seeded_quick_shape_slots);
        let editable_curve = self
            .sound_states
            .read()
            .ok()
            .and_then(|states| states.get(index).map(|state| state.editable_curve.clone()))
            .unwrap_or_else(default_editable_curve);
        if let Ok(mut states) = self.sound_states.write() {
            states[index] = PumpSoundState {
                mix: self.realtime_mix[index].load(Ordering::Acquire),
                depth_db: self.realtime_depth_db[index].load(Ordering::Acquire),
                floor_db: self.realtime_floor_db[index].load(Ordering::Acquire),
                phase_offset: self.realtime_phase_offset[index].load(Ordering::Acquire),
                output_gain_db: self.realtime_output_gain_db[index].load(Ordering::Acquire),
                sync_division: self.realtime_sync_division[index].load(Ordering::Acquire) as usize,
                trigger_mode: self.realtime_trigger_mode[index].load(Ordering::Acquire) as usize,
                smooth: self.realtime_smooth[index].load(Ordering::Acquire),
                mode: self.realtime_mode[index].load(Ordering::Acquire) as usize,
                swing: self.realtime_swing[index].load(Ordering::Acquire),
                editable_curve,
                quick_slots,
            };
            self.active_sound_dirty.store(false, Ordering::Release);
        }
    }

    fn mark_active_sound_dirty(&self) {
        self.active_sound_dirty.store(true, Ordering::Release);
    }

    fn publish_sound_state_at(&self, index: usize, state: &PumpSoundState) {
        let active_index = self.realtime_index();
        if index == active_index {
            self.mix
                .store(state.mix.clamp(MIN_MIX, MAX_MIX), Ordering::Relaxed);
        }
        self.realtime_mix[index].store(state.mix.clamp(MIN_MIX, MAX_MIX), Ordering::Relaxed);
        if index == active_index {
            self.depth_db.store(
                state.depth_db.clamp(MIN_DEPTH_DB, MAX_DEPTH_DB),
                Ordering::Relaxed,
            );
        }
        self.realtime_depth_db[index].store(
            state.depth_db.clamp(MIN_DEPTH_DB, MAX_DEPTH_DB),
            Ordering::Relaxed,
        );
        if index == active_index {
            self.floor_db.store(
                state.floor_db.clamp(MIN_FLOOR_DB, MAX_FLOOR_DB),
                Ordering::Relaxed,
            );
        }
        self.realtime_floor_db[index].store(
            state.floor_db.clamp(MIN_FLOOR_DB, MAX_FLOOR_DB),
            Ordering::Relaxed,
        );
        if index == active_index {
            self.phase_offset.store(
                state.phase_offset.clamp(MIN_PHASE_OFFSET, MAX_PHASE_OFFSET),
                Ordering::Relaxed,
            );
        }
        self.realtime_phase_offset[index].store(
            state.phase_offset.clamp(MIN_PHASE_OFFSET, MAX_PHASE_OFFSET),
            Ordering::Relaxed,
        );
        if index == active_index {
            self.output_gain_db.store(
                state
                    .output_gain_db
                    .clamp(MIN_OUTPUT_GAIN_DB, MAX_OUTPUT_GAIN_DB),
                Ordering::Relaxed,
            );
        }
        self.realtime_output_gain_db[index].store(
            state
                .output_gain_db
                .clamp(MIN_OUTPUT_GAIN_DB, MAX_OUTPUT_GAIN_DB),
            Ordering::Relaxed,
        );
        if index == active_index {
            self.sync_division.store(
                clamp_sync_division(state.sync_division as f32) as u32,
                Ordering::Relaxed,
            );
        }
        self.realtime_sync_division[index].store(
            clamp_sync_division(state.sync_division as f32) as u32,
            Ordering::Relaxed,
        );
        if index == active_index {
            self.trigger_mode.store(
                clamp_trigger_mode(state.trigger_mode as f32) as u32,
                Ordering::Relaxed,
            );
        }
        self.realtime_trigger_mode[index].store(
            clamp_trigger_mode(state.trigger_mode as f32) as u32,
            Ordering::Relaxed,
        );
        if index == active_index {
            self.smooth.store(
                state.smooth.clamp(MIN_SMOOTH, MAX_SMOOTH),
                Ordering::Relaxed,
            );
        }
        self.realtime_smooth[index].store(
            state.smooth.clamp(MIN_SMOOTH, MAX_SMOOTH),
            Ordering::Relaxed,
        );
        if index == active_index {
            self.mode.store(
                clamp_processing_mode(state.mode as f32) as u32,
                Ordering::Relaxed,
            );
        }
        self.realtime_mode[index].store(
            clamp_processing_mode(state.mode as f32) as u32,
            Ordering::Relaxed,
        );
        if index == active_index {
            self.swing
                .store(state.swing.clamp(MIN_SWING, MAX_SWING), Ordering::Relaxed);
        }
        self.realtime_swing[index]
            .store(state.swing.clamp(MIN_SWING, MAX_SWING), Ordering::Relaxed);
        let normalized = state.editable_curve.clone().normalized();
        let curve_table = editable_curve_to_table(&normalized);
        for (curve, value) in self.realtime_curve[index]
            .iter()
            .zip(curve_table.iter().copied())
        {
            curve.store(value.clamp(0.0, 1.0), Ordering::Release);
        }
        if index == active_index {
            if let Ok(mut guard) = self.editable_curve.write() {
                *guard = normalized;
            }
            self.store_curve_table(&curve_table);
            self.curve_revision.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn reconcile_active_sound_projection(&self) {
        let side = self.active_sound();
        let Some(state) = self
            .sound_states
            .read()
            .ok()
            .and_then(|states| states.get(side.index()).cloned())
        else {
            return;
        };
        let curve_table = editable_curve_to_table(&state.editable_curve);
        if let Ok(mut guard) = self.editable_curve.write() {
            *guard = state.editable_curve;
        }
        for (index, sample) in curve_table.iter().copied().enumerate() {
            self.curve[index].store(sample.clamp(0.0, 1.0), Ordering::Release);
        }
        self.active_sound_dirty.store(false, Ordering::Release);
    }

    fn current_preset_snapshot_with_name(&self, name: String) -> PumpPreset {
        PumpPreset {
            name,
            is_read_only: false,
            is_favorite: false,
            mix: self.mix(),
            depth: self.depth(),
            depth_db: self.depth_db(),
            floor_db: self.floor_db(),
            phase_offset: self.phase_offset(),
            output_gain_db: self.output_gain_db(),
            sync_division: self.sync_division(),
            trigger_mode: self.trigger_mode(),
            smooth: self.smooth(),
            mode: self.mode(),
            swing: self.swing(),
            editable_curve: self.editable_curve_snapshot(),
            quick_slots: self.sound_state_snapshot(self.active_sound()).quick_slots,
        }
    }

    fn log_preset_persistence_error(context: &str, error: &str) {
        eprintln!("pump preset persistence {context} failed: {error}");
    }

    fn record_preset_persistence_failure(&self, context: &str, error: &str) {
        Self::log_preset_persistence_error(context, error);
        if let Ok(mut warning) = self.preset_persistence_warning.write() {
            *warning = Some(error.to_string());
        }
    }

    fn persist_preset_bank_snapshot(
        &self,
        bank: &PumpPresetBank,
    ) -> Result<(), PresetMutationError> {
        match super::preset_store::persist_preset_bank(bank) {
            Ok(()) => {
                if let Ok(mut warning) = self.preset_persistence_warning.write() {
                    *warning = None;
                }
                Ok(())
            }
            Err(message) => {
                self.record_preset_persistence_failure("save", message.as_str());
                Err(PresetMutationError::PersistenceFailed { message })
            }
        }
    }

    fn normalized_preset_bank(&self, mut bank: PumpPresetBank) -> PumpPresetBank {
        if bank.presets.is_empty() {
            bank = PumpPresetBank::default_init();
        }
        if bank.presets.len() > MAX_PRESETS {
            bank.presets.truncate(MAX_PRESETS);
        }
        for (index, preset) in bank.presets.iter_mut().enumerate() {
            preset.name = sanitize_preset_name(&preset.name, index);
            // Persist the field for backward-compatible serialization, but keep
            // runtime behavior fully writable across all presets.
            preset.is_read_only = false;
            // Keep the legacy compatibility field at its historical full-depth
            // value; new state uses depth_db as the authoritative field.
            preset.depth = DEFAULT_DEPTH;
            preset.depth_db = if preset.depth_db.is_finite() {
                preset.depth_db.clamp(MIN_DEPTH_DB, MAX_DEPTH_DB)
            } else {
                DEFAULT_DEPTH_DB
            };
            preset.floor_db = if preset.floor_db.is_finite() {
                preset.floor_db.clamp(MIN_FLOOR_DB, MAX_FLOOR_DB)
            } else {
                DEFAULT_FLOOR_DB
            };
            preset.sync_division = preset.sync_division.min(MAX_SYNC_DIVISION as usize);
            preset.trigger_mode = clamp_trigger_mode(preset.trigger_mode as f32);
            preset.mode = clamp_processing_mode(preset.mode as f32);
            preset.swing = if preset.swing.is_finite() {
                preset.swing.clamp(MIN_SWING, MAX_SWING)
            } else {
                DEFAULT_SWING
            };
            preset.smooth = if preset.smooth.is_finite() {
                preset.smooth.clamp(MIN_SMOOTH, MAX_SMOOTH)
            } else {
                DEFAULT_SMOOTH
            };
            preset.editable_curve = preset.editable_curve.clone().normalized();
            let mut normalized_slots = preset.quick_slots.clone();
            if normalized_slots.len() > QUICK_SLOT_COUNT {
                normalized_slots.truncate(QUICK_SLOT_COUNT);
            }
            let seed_slots = seeded_quick_shape_slots();
            for (slot_index, seed_slot) in seed_slots.into_iter().enumerate() {
                let slot = normalized_slots.get_mut(slot_index);
                if let Some(slot) = slot {
                    slot.curve = slot.curve.clone().normalized();
                } else {
                    normalized_slots.push(seed_slot);
                }
            }
            preset.quick_slots = normalized_slots;
        }
        bank.selected = bank.selected.min(bank.presets.len().saturating_sub(1));
        bank
    }

    fn apply_preset_snapshot(&self, preset: &PumpPreset) {
        self.set_mix(preset.mix);
        self.set_depth_db(preset.depth_db);
        self.set_floor_db(preset.floor_db);
        self.set_phase_offset(preset.phase_offset);
        self.set_output_gain_db(preset.output_gain_db);
        self.set_sync_division(preset.sync_division as f32);
        self.set_trigger_mode(preset.trigger_mode as f32);
        self.set_smooth(preset.smooth);
        self.set_mode(preset.mode as f32);
        self.set_swing(preset.swing);
        self.set_editable_curve_preserving_phase(&preset.editable_curve);
        let _ = self.set_active_sound_quick_slots(preset.quick_slots.clone());
    }

    /// Snapshot the stored preset bank.
    pub fn preset_bank_snapshot(&self) -> PumpPresetBank {
        self.preset_bank
            .read()
            .map(|bank| bank.clone())
            .unwrap_or_else(|_| PumpPresetBank::default_init())
    }

    /// Snapshot the selected preset's quick slots.
    pub fn selected_quick_slots_snapshot(&self) -> Vec<QuickShapeSlot> {
        self.sound_state_snapshot(self.active_sound()).quick_slots
    }

    /// Snapshot one quick-slot curve from the selected preset.
    pub fn selected_quick_slot_curve(&self, index: usize) -> Option<EditableCurve> {
        self.selected_quick_slots_snapshot()
            .get(index)
            .map(|slot| slot.curve.clone())
    }

    /// Replace one quick-slot curve on the selected preset and persist it.
    pub fn set_selected_quick_slot_curve(
        &self,
        index: usize,
        curve: &EditableCurve,
    ) -> Result<(), PresetMutationError> {
        let Ok(mut guard) = self.preset_bank.write() else {
            return Err(PresetMutationError::StateUnavailable);
        };
        let mut candidate = guard.clone();
        let selected = candidate
            .selected
            .min(candidate.presets.len().saturating_sub(1));
        let Some(preset) = candidate.presets.get_mut(selected) else {
            return Err(PresetMutationError::InvalidIndex);
        };
        let Some(slot) = preset.quick_slots.get_mut(index) else {
            return Err(PresetMutationError::InvalidIndex);
        };
        let normalized_curve = curve.clone().normalized();
        slot.curve = normalized_curve.clone();
        self.persist_preset_bank_snapshot(&candidate)?;
        *guard = candidate;
        drop(guard);
        let mut active_slots = self.selected_quick_slots_snapshot();
        if let Some(active_slot) = active_slots.get_mut(index) {
            active_slot.curve = normalized_curve;
        }
        let _ = self.set_active_sound_quick_slots(active_slots);
        Ok(())
    }

    /// Snapshot the globally persisted curve slots.
    pub fn global_curve_slots_snapshot(&self) -> Vec<GlobalCurveSlot> {
        match super::global_curve_slots::load_global_curve_slots() {
            Ok(slots) => slots,
            Err(error) => {
                Self::log_preset_persistence_error("load global curve slots", error.as_str());
                vec![GlobalCurveSlot { curve: None }; GLOBAL_CURVE_SLOT_COUNT]
            }
        }
    }

    /// Snapshot one globally persisted curve slot.
    pub fn global_curve_slot_curve(&self, index: usize) -> Option<EditableCurve> {
        self.global_curve_slots_snapshot()
            .get(index)
            .and_then(|slot| slot.curve.clone())
    }

    /// Store the current editable curve into one globally persisted slot.
    pub fn set_global_curve_slot_curve(&self, index: usize, curve: &EditableCurve) -> bool {
        match super::global_curve_slots::store_global_curve_slot(index, curve) {
            Ok(()) => true,
            Err(error) => {
                Self::log_preset_persistence_error("save global curve slot", error.as_str());
                false
            }
        }
    }

    /// Return whether the current editable curve differs from a stored global slot.
    pub fn current_curve_deviates_from_global_slot(&self, index: usize) -> bool {
        let Some(slot_curve) = self.global_curve_slot_curve(index) else {
            return false;
        };
        !curve_near_eq(&self.editable_curve_snapshot(), &slot_curve)
    }

    /// Replace the full preset bank, clamping to supported limits.
    pub fn set_preset_bank(&self, bank: PumpPresetBank) -> Result<(), PresetMutationError> {
        let normalized = self.normalized_preset_bank(bank);
        let Ok(mut guard) = self.preset_bank.write() else {
            return Err(PresetMutationError::StateUnavailable);
        };
        self.persist_preset_bank_snapshot(&normalized)?;
        *guard = normalized;
        Ok(())
    }

    /// Replace the full preset bank without touching persistent disk storage.
    pub(crate) fn set_preset_bank_without_persistence(&self, bank: PumpPresetBank) {
        let normalized = self.normalized_preset_bank(bank);
        if let Ok(mut guard) = self.preset_bank.write() {
            *guard = normalized;
        }
    }

    /// Restore the complete A/B bank after a state payload has been validated.
    pub(crate) fn set_sound_states_without_persistence(
        &self,
        active: SoundSide,
        states: [PumpSoundState; 2],
    ) {
        self.pending_active_sound.store(u32::MAX, Ordering::Release);
        self.active_sound_dirty.store(false, Ordering::Release);
        if let Ok(mut guard) = self.sound_states.write() {
            *guard = states;
        }
        for side in [SoundSide::A, SoundSide::B] {
            let snapshot = self
                .sound_states
                .read()
                .ok()
                .and_then(|states| states.get(side.index()).cloned())
                .unwrap_or_else(PumpSoundState::default_init);
            self.publish_sound_state_at(side.index(), &snapshot);
        }
        self.active_sound
            .store(active.index() as u32, Ordering::Release);
        self.curve_revision.fetch_add(1, Ordering::AcqRel);
        self.reconcile_active_sound_projection();
    }

    /// Load one preset by index into the active parameter state.
    pub fn load_preset(&self, index: usize) -> Result<usize, PresetMutationError> {
        let Ok(mut guard) = self.preset_bank.write() else {
            return Err(PresetMutationError::StateUnavailable);
        };
        let Some(preset) = guard.presets.get(index).cloned() else {
            return Err(PresetMutationError::InvalidIndex);
        };
        let mut candidate = guard.clone();
        candidate.selected = index;
        self.persist_preset_bank_snapshot(&candidate)?;
        *guard = candidate;
        drop(guard);
        self.apply_preset_snapshot(&preset);
        Ok(index)
    }

    /// Load a preset relative to the current selection, clamping at the bank
    /// boundaries instead of wrapping around.
    pub fn load_preset_relative(&self, direction: i32) -> Result<usize, PresetMutationError> {
        let bank = self.preset_bank_snapshot();
        let Some(last) = bank.presets.len().checked_sub(1) else {
            return Err(PresetMutationError::InvalidIndex);
        };
        let selected = bank.selected.min(last);
        let target = if direction < 0 {
            selected.saturating_sub(1)
        } else if direction > 0 {
            selected.saturating_add(1).min(last)
        } else {
            selected
        };
        if target == selected {
            return Ok(selected);
        }
        self.load_preset(target)
    }

    /// Toggle the selected preset's favorite marker and persist the bank.
    pub fn toggle_selected_preset_favorite(&self) -> Result<bool, PresetMutationError> {
        let selected = self.preset_bank_snapshot().selected;
        self.set_preset_favorite(selected, None)
    }

    /// Set or toggle a preset's favorite marker and persist the bank.
    pub fn set_preset_favorite(
        &self,
        index: usize,
        value: Option<bool>,
    ) -> Result<bool, PresetMutationError> {
        let Ok(mut guard) = self.preset_bank.write() else {
            return Err(PresetMutationError::StateUnavailable);
        };
        let mut candidate = guard.clone();
        let Some(preset) = candidate.presets.get_mut(index) else {
            return Err(PresetMutationError::InvalidIndex);
        };
        preset.is_favorite = value.unwrap_or(!preset.is_favorite);
        let favorite = preset.is_favorite;
        self.persist_preset_bank_snapshot(&candidate)?;
        *guard = candidate;
        Ok(favorite)
    }

    /// Insert a new preset cloned from current state and select it.
    pub fn add_preset_from_current_state(&self) -> Result<usize, PresetMutationError> {
        let snapshot = self.current_preset_snapshot_with_name(String::new());
        let Ok(mut guard) = self.preset_bank.write() else {
            return Err(PresetMutationError::StateUnavailable);
        };
        if guard.presets.len() >= MAX_PRESETS {
            return Err(PresetMutationError::CapacityReached);
        }
        let insert_at = guard.selected.saturating_add(1).min(guard.presets.len());
        let fallback_index = guard.presets.len();
        let mut inserted = snapshot;
        inserted.name = sanitize_preset_name("", fallback_index);
        inserted.is_read_only = false;
        let mut candidate = guard.clone();
        candidate.presets.insert(insert_at, inserted);
        candidate.selected = insert_at;
        self.persist_preset_bank_snapshot(&candidate)?;
        *guard = candidate;
        Ok(insert_at)
    }

    /// Rename one preset entry.
    pub fn rename_preset(&self, index: usize, new_name: &str) -> Result<(), PresetMutationError> {
        let Ok(mut guard) = self.preset_bank.write() else {
            return Err(PresetMutationError::StateUnavailable);
        };
        let mut candidate_bank = guard.clone();
        let Some(preset) = candidate_bank.presets.get_mut(index) else {
            return Err(PresetMutationError::InvalidIndex);
        };
        let candidate = sanitize_preset_name(new_name, index);
        preset.name = candidate;
        self.persist_preset_bank_snapshot(&candidate_bank)?;
        *guard = candidate_bank;
        Ok(())
    }

    /// Save current state by preset name using overwrite-or-create semantics.
    pub fn save_current_state_by_name(
        &self,
        name: &str,
    ) -> Result<SavePresetOutcome, PresetMutationError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(PresetMutationError::InvalidName);
        }
        let candidate_name: String = trimmed.chars().take(MAX_PRESET_NAME_CHARS).collect();
        if candidate_name.is_empty() {
            return Err(PresetMutationError::InvalidName);
        }
        let normalized_candidate = normalized_preset_name(&candidate_name);

        let snapshot = self.current_preset_snapshot_with_name(candidate_name.clone());
        let Ok(mut guard) = self.preset_bank.write() else {
            return Err(PresetMutationError::StateUnavailable);
        };
        let mut candidate_bank = guard.clone();

        let matching_index = candidate_bank
            .presets
            .iter()
            .position(|preset| normalized_preset_name(&preset.name) == normalized_candidate);
        if let Some(index) = matching_index {
            if let Some(existing) = candidate_bank.presets.get_mut(index) {
                existing.mix = snapshot.mix;
                existing.depth = snapshot.depth;
                existing.depth_db = snapshot.depth_db;
                existing.floor_db = snapshot.floor_db;
                existing.phase_offset = snapshot.phase_offset;
                existing.output_gain_db = snapshot.output_gain_db;
                existing.sync_division = snapshot.sync_division;
                existing.trigger_mode = snapshot.trigger_mode;
                existing.smooth = snapshot.smooth;
                existing.mode = snapshot.mode;
                existing.swing = snapshot.swing;
                existing.editable_curve = snapshot.editable_curve;
                existing.quick_slots = snapshot.quick_slots;
            }
            candidate_bank.selected = index;
            self.persist_preset_bank_snapshot(&candidate_bank)?;
            *guard = candidate_bank;
            return Ok(SavePresetOutcome::Overwritten { index });
        }

        if candidate_bank.presets.len() >= MAX_PRESETS {
            return Err(PresetMutationError::CapacityReached);
        }
        let created_index = candidate_bank.presets.len();
        candidate_bank.presets.push(snapshot);
        candidate_bank.selected = created_index;
        self.persist_preset_bank_snapshot(&candidate_bank)?;
        *guard = candidate_bank;
        Ok(SavePresetOutcome::Created {
            index: created_index,
        })
    }

    /// Return the most recent preset-store failure until a later durable write succeeds.
    pub fn preset_persistence_warning(&self) -> Option<String> {
        self.preset_persistence_warning
            .read()
            .ok()
            .and_then(|warning| warning.clone())
    }

    /// Return true when current parameters/curve differ from selected preset.
    pub fn current_state_differs_from_selected_preset(&self) -> bool {
        let bank = self.preset_bank_snapshot();
        let Some(selected) = bank.presets.get(bank.selected) else {
            return false;
        };
        let current = self.current_preset_snapshot_with_name(String::new());
        !float_near_eq(current.mix, selected.mix)
            || !float_near_eq(current.depth_db, selected.depth_db)
            || !float_near_eq(current.floor_db, selected.floor_db)
            || !float_near_eq(current.phase_offset, selected.phase_offset)
            || !float_near_eq(current.output_gain_db, selected.output_gain_db)
            || current.sync_division != selected.sync_division
            || current.trigger_mode != selected.trigger_mode
            || !float_near_eq(current.smooth, selected.smooth)
            || current.mode != selected.mode
            || !float_near_eq(current.swing, selected.swing)
            || !curve_near_eq(&current.editable_curve, &selected.editable_curve)
    }

    fn store_curve_table(&self, values: &[f32; CURVE_TABLE_LEN]) {
        let active_curve = &self.realtime_curve[self.realtime_index()];
        for (index, sample) in values.iter().copied().enumerate() {
            let sample = sample.clamp(0.0, 1.0);
            self.curve[index].store(sample, Ordering::Release);
            active_curve[index].store(sample, Ordering::Release);
        }
    }
}

impl Default for PumpParams {
    fn default() -> Self {
        Self::new()
    }
}
