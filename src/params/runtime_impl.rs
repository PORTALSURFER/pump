use super::*;
impl PumpParams {
    /// Create params with production defaults and default curve.
    pub fn new() -> Self {
        let editable_curve = default_editable_curve();
        let default_curve = editable_curve_to_table(&editable_curve);
        let params = Self {
            mix: AtomicF32::new(DEFAULT_MIX),
            depth: AtomicF32::new(DEFAULT_DEPTH),
            phase_offset: AtomicF32::new(DEFAULT_PHASE_OFFSET),
            output_gain_db: AtomicF32::new(DEFAULT_OUTPUT_GAIN_DB),
            sync_division: AtomicU32::new(DEFAULT_SYNC_DIVISION_INDEX as u32),
            editable_curve: RwLock::new(editable_curve),
            curve: std::array::from_fn(|index| AtomicF32::new(default_curve[index])),
            curve_revision: AtomicU32::new(1),
            preset_bank: RwLock::new(PumpPresetBank::default_init()),
        };
        match super::preset_store::load_persisted_preset_bank() {
            Ok(Some(bank)) => params.set_preset_bank_without_persistence(bank),
            Ok(None) => {}
            Err(error) => Self::log_preset_persistence_error("load", error.as_str()),
        }
        params
    }

    /// Get dry/wet mix amount.
    pub fn mix(&self) -> f32 {
        self.mix.load(Ordering::Relaxed)
    }

    /// Get duck depth amount.
    pub fn depth(&self) -> f32 {
        self.depth.load(Ordering::Relaxed)
    }

    /// Get cycle phase offset.
    pub fn phase_offset(&self) -> f32 {
        self.phase_offset.load(Ordering::Relaxed)
    }

    /// Get output gain in decibels.
    pub fn output_gain_db(&self) -> f32 {
        self.output_gain_db.load(Ordering::Relaxed)
    }

    /// Get sync division index.
    pub fn sync_division(&self) -> usize {
        clamp_sync_division(self.sync_division.load(Ordering::Relaxed) as f32)
    }

    /// Get sync division in beats per cycle.
    pub fn sync_beats_per_cycle(&self) -> f32 {
        sync_division_beats(self.sync_division())
    }

    /// Set dry/wet mix amount.
    pub fn set_mix(&self, value: f32) {
        self.mix
            .store(value.clamp(MIN_MIX, MAX_MIX), Ordering::Relaxed);
    }

    /// Set duck depth amount.
    pub fn set_depth(&self, value: f32) {
        self.depth
            .store(value.clamp(MIN_DEPTH, MAX_DEPTH), Ordering::Relaxed);
    }

    /// Set cycle phase offset.
    pub fn set_phase_offset(&self, value: f32) {
        self.phase_offset.store(
            value.clamp(MIN_PHASE_OFFSET, MAX_PHASE_OFFSET),
            Ordering::Relaxed,
        );
    }

    /// Set output gain in decibels.
    pub fn set_output_gain_db(&self, value: f32) {
        self.output_gain_db.store(
            value.clamp(MIN_OUTPUT_GAIN_DB, MAX_OUTPUT_GAIN_DB),
            Ordering::Relaxed,
        );
    }

    /// Set sync division index from scalar host value.
    pub fn set_sync_division(&self, value: f32) {
        let index = clamp_sync_division(value);
        self.sync_division.store(index as u32, Ordering::Relaxed);
    }

    /// Read the current curve revision counter.
    pub fn curve_revision(&self) -> u32 {
        self.curve_revision.load(Ordering::Acquire)
    }

    /// Read one point from the curve table.
    pub fn curve_value(&self, index: usize) -> f32 {
        self.curve
            .get(index)
            .map(|sample: &AtomicF32| sample.load(Ordering::Acquire).clamp(0.0, 1.0))
            .unwrap_or(1.0)
    }

    /// Snapshot the whole curve table in one array.
    pub fn curve_snapshot(&self) -> [f32; CURVE_TABLE_LEN] {
        let mut values = [1.0_f32; CURVE_TABLE_LEN];
        for (index, value) in values.iter_mut().enumerate() {
            *value = self.curve[index].load(Ordering::Acquire).clamp(0.0, 1.0);
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
        let normalized = editable_curve.clone().normalized();
        let curve_table = editable_curve_to_table(&normalized);
        if let Ok(mut guard) = self.editable_curve.write() {
            *guard = normalized;
        }
        self.store_curve_table(&curve_table);
        self.curve_revision.fetch_add(1, Ordering::AcqRel);
    }

    /// Replace the whole curve table and advance revision.
    pub fn set_curve(&self, values: &[f32; CURVE_TABLE_LEN]) {
        if let Ok(mut guard) = self.editable_curve.write() {
            *guard = curve_table_to_editable(values);
        }
        self.store_curve_table(values);
        self.curve_revision.fetch_add(1, Ordering::AcqRel);
    }

    /// Restore default curve shape and advance revision.
    pub fn reset_curve_to_default(&self) {
        self.set_editable_curve(&default_editable_curve());
    }

    fn current_preset_snapshot_with_name(&self, name: String) -> PumpPreset {
        PumpPreset {
            name,
            is_read_only: false,
            mix: self.mix(),
            depth: self.depth(),
            phase_offset: self.phase_offset(),
            output_gain_db: self.output_gain_db(),
            sync_division: self.sync_division(),
            editable_curve: self.editable_curve_snapshot(),
        }
    }

    fn log_preset_persistence_error(context: &str, error: &str) {
        eprintln!("pump preset persistence {context} failed: {error}");
    }

    fn persist_preset_bank_snapshot(&self, bank: &PumpPresetBank) {
        if let Err(error) = super::preset_store::persist_preset_bank(bank) {
            Self::log_preset_persistence_error("save", error.as_str());
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
            preset.sync_division = preset.sync_division.min(MAX_SYNC_DIVISION as usize);
            preset.editable_curve = preset.editable_curve.clone().normalized();
        }
        bank.selected = bank.selected.min(bank.presets.len().saturating_sub(1));
        bank
    }

    fn apply_preset_snapshot(&self, preset: &PumpPreset) {
        self.set_mix(preset.mix);
        self.set_depth(preset.depth);
        self.set_phase_offset(preset.phase_offset);
        self.set_output_gain_db(preset.output_gain_db);
        self.set_sync_division(preset.sync_division as f32);
        self.set_editable_curve(&preset.editable_curve);
    }

    /// Snapshot the stored preset bank.
    pub fn preset_bank_snapshot(&self) -> PumpPresetBank {
        self.preset_bank
            .read()
            .map(|bank| bank.clone())
            .unwrap_or_else(|_| PumpPresetBank::default_init())
    }

    /// Replace the full preset bank, clamping to supported limits.
    pub fn set_preset_bank(&self, bank: PumpPresetBank) {
        let normalized = self.normalized_preset_bank(bank);
        if let Ok(mut guard) = self.preset_bank.write() {
            *guard = normalized.clone();
        }
        self.persist_preset_bank_snapshot(&normalized);
    }

    /// Replace the full preset bank without touching persistent disk storage.
    pub(crate) fn set_preset_bank_without_persistence(&self, bank: PumpPresetBank) {
        let normalized = self.normalized_preset_bank(bank);
        if let Ok(mut guard) = self.preset_bank.write() {
            *guard = normalized;
        }
    }

    /// Load one preset by index into the active parameter state.
    pub fn load_preset(&self, index: usize) -> Option<usize> {
        let preset = self
            .preset_bank
            .read()
            .ok()
            .and_then(|bank| bank.presets.get(index).cloned())?;
        self.apply_preset_snapshot(&preset);
        let mut persisted = None;
        if let Ok(mut guard) = self.preset_bank.write() {
            guard.selected = index.min(guard.presets.len().saturating_sub(1));
            let selected = guard.selected;
            persisted = Some((selected, guard.clone()));
        }
        if let Some((selected, persisted_bank)) = persisted {
            self.persist_preset_bank_snapshot(&persisted_bank);
            return Some(selected);
        }
        Some(index)
    }

    /// Insert a new preset cloned from current state and select it.
    pub fn add_preset_from_current_state(&self) -> Option<usize> {
        let snapshot = self.current_preset_snapshot_with_name(String::new());
        let Ok(mut guard) = self.preset_bank.write() else {
            return None;
        };
        if guard.presets.len() >= MAX_PRESETS {
            return None;
        }
        let insert_at = guard.selected.saturating_add(1).min(guard.presets.len());
        let fallback_index = guard.presets.len();
        let mut inserted = snapshot;
        inserted.name = sanitize_preset_name("", fallback_index);
        inserted.is_read_only = false;
        guard.presets.insert(insert_at, inserted);
        guard.selected = insert_at;
        let persisted = guard.clone();
        drop(guard);
        self.persist_preset_bank_snapshot(&persisted);
        Some(insert_at)
    }

    /// Rename one preset entry.
    pub fn rename_preset(&self, index: usize, new_name: &str) -> bool {
        let Ok(mut guard) = self.preset_bank.write() else {
            return false;
        };
        let Some(preset) = guard.presets.get_mut(index) else {
            return false;
        };
        let candidate = sanitize_preset_name(new_name, index);
        preset.name = candidate;
        let persisted = guard.clone();
        drop(guard);
        self.persist_preset_bank_snapshot(&persisted);
        true
    }

    /// Save current state by preset name using overwrite-or-create semantics.
    pub fn save_current_state_by_name(&self, name: &str) -> SavePresetOutcome {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return SavePresetOutcome::InvalidName;
        }
        let candidate_name: String = trimmed.chars().take(MAX_PRESET_NAME_CHARS).collect();
        if candidate_name.is_empty() {
            return SavePresetOutcome::InvalidName;
        }
        let normalized_candidate = normalized_preset_name(&candidate_name);

        let snapshot = self.current_preset_snapshot_with_name(candidate_name.clone());
        let Ok(mut guard) = self.preset_bank.write() else {
            return SavePresetOutcome::InvalidName;
        };

        let matching_index = guard
            .presets
            .iter()
            .position(|preset| normalized_preset_name(&preset.name) == normalized_candidate);
        if let Some(index) = matching_index {
            if let Some(existing) = guard.presets.get_mut(index) {
                existing.mix = snapshot.mix;
                existing.depth = snapshot.depth;
                existing.phase_offset = snapshot.phase_offset;
                existing.output_gain_db = snapshot.output_gain_db;
                existing.sync_division = snapshot.sync_division;
                existing.editable_curve = snapshot.editable_curve;
            }
            guard.selected = index;
            let persisted = guard.clone();
            drop(guard);
            self.persist_preset_bank_snapshot(&persisted);
            return SavePresetOutcome::Overwritten { index };
        }

        if guard.presets.len() >= MAX_PRESETS {
            return SavePresetOutcome::BlockedFull;
        }
        let created_index = guard.presets.len();
        guard.presets.push(snapshot);
        guard.selected = created_index;
        let persisted = guard.clone();
        drop(guard);
        self.persist_preset_bank_snapshot(&persisted);
        SavePresetOutcome::Created {
            index: created_index,
        }
    }

    /// Return true when current parameters/curve differ from selected preset.
    pub fn current_state_differs_from_selected_preset(&self) -> bool {
        let bank = self.preset_bank_snapshot();
        let Some(selected) = bank.presets.get(bank.selected) else {
            return false;
        };
        let current = self.current_preset_snapshot_with_name(String::new());
        !float_near_eq(current.mix, selected.mix)
            || !float_near_eq(current.depth, selected.depth)
            || !float_near_eq(current.phase_offset, selected.phase_offset)
            || !float_near_eq(current.output_gain_db, selected.output_gain_db)
            || current.sync_division != selected.sync_division
            || !curve_near_eq(&current.editable_curve, &selected.editable_curve)
    }

    fn store_curve_table(&self, values: &[f32; CURVE_TABLE_LEN]) {
        for (index, sample) in values.iter().copied().enumerate() {
            self.curve[index].store(sample.clamp(0.0, 1.0), Ordering::Release);
        }
    }
}

impl Default for PumpParams {
    fn default() -> Self {
        Self::new()
    }
}
