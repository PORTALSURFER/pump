//! Lock-free, cycle-aligned incoming-audio visualization snapshots.

use std::array;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::time_utils::monotonic_micros;

/// Fixed horizontal resolution shared by capture and both editor renderers.
pub(crate) const INCOMING_WAVEFORM_BIN_COUNT: usize = 96;
const SNAPSHOT_STALE_AFTER_MICROS: u64 = 750_000;
const SNAPSHOT_READ_RETRIES: usize = 8;
const SILENCE_FLOOR: f32 = 1.0e-4;
const BACKWARD_PHASE_EPSILON: f32 = 1.0e-5;
const FORWARD_PHASE_DISCONTINUITY: f32 = 0.05;

/// A bounded GUI snapshot of peak input amplitude over one normalized cycle.
pub(crate) type IncomingWaveformSnapshot = [f32; INCOMING_WAVEFORM_BIN_COUNT];

fn pack_slot(generation: u32, value_bits: u32) -> u64 {
    (u64::from(generation) << 32) | u64::from(value_bits)
}

fn slot_generation(slot: u64) -> u32 {
    (slot >> 32) as u32
}

fn slot_value_bits(slot: u64) -> u32 {
    slot as u32
}

fn pack_display_token(page: usize, generation: u32) -> u64 {
    (u64::from(generation) << 1) | (page as u64 & 1)
}

fn unpack_display_token(token: u64) -> Option<(usize, u32)> {
    let generation = (token >> 1) as u32;
    (generation != 0).then_some(((token & 1) as usize, generation))
}

struct IncomingWaveformPage {
    /// Even means stable for readers; odd means the writer owns the page.
    sequence: AtomicU32,
    /// Packed generation and float bits make each slot a single atomic state.
    slots: [AtomicU64; INCOMING_WAVEFORM_BIN_COUNT],
}

impl Default for IncomingWaveformPage {
    fn default() -> Self {
        Self {
            sequence: AtomicU32::new(0),
            slots: array::from_fn(|_| AtomicU64::new(pack_slot(0, 0.0_f32.to_bits()))),
        }
    }
}

/// Shared atomics written by the audio thread and sampled by the GUI thread.
pub(crate) struct IncomingWaveformBuffer {
    generation: AtomicU32,
    /// Zero means no display is currently valid; otherwise this names a page and generation.
    display_token: AtomicU64,
    live_mode: AtomicBool,
    pages: [IncomingWaveformPage; 2],
    last_update_micros: AtomicU64,
}

impl Default for IncomingWaveformBuffer {
    fn default() -> Self {
        Self {
            generation: AtomicU32::new(1),
            // The first page is published after its first block is complete. Subsequent
            // writes use the other page so no displayed page is mutated in place.
            display_token: AtomicU64::new(0),
            live_mode: AtomicBool::new(false),
            pages: array::from_fn(|_| IncomingWaveformPage::default()),
            last_update_micros: AtomicU64::new(0),
        }
    }
}

impl IncomingWaveformBuffer {
    fn generation(&self) -> u32 {
        self.generation.load(Ordering::Acquire)
    }

    fn begin_next_generation(&self) -> u32 {
        let next = self
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
            .max(1);
        if next == 1 {
            self.generation.store(1, Ordering::Release);
        }
        next
    }

    fn displayed_page(&self) -> Option<usize> {
        unpack_display_token(self.display_token.load(Ordering::Acquire)).map(|(page, _)| page)
    }

    fn writable_page(&self, preferred: usize) -> usize {
        let preferred = preferred & 1;
        if self.displayed_page() == Some(preferred) {
            preferred ^ 1
        } else {
            preferred
        }
    }

    /// Claim a page for mutation and initialize every slot before any data is published.
    fn begin_page_write(&self, page: usize, generation: u32, seed_from_display: bool) {
        let page_state = &self.pages[page & 1];
        if page_state.sequence.load(Ordering::Acquire) & 1 == 0 {
            page_state.sequence.fetch_add(1, Ordering::AcqRel);
        }

        let displayed = unpack_display_token(self.display_token.load(Ordering::Acquire));
        for (index, slot) in page_state.slots.iter().enumerate() {
            let existing = slot.load(Ordering::Relaxed);
            let value_bits = if seed_from_display && slot_generation(existing) == generation {
                slot_value_bits(existing)
            } else if let Some((displayed_page, displayed_generation)) = displayed {
                let source = self.pages[displayed_page].slots[index].load(Ordering::Acquire);
                if slot_generation(source) == displayed_generation {
                    slot_value_bits(source)
                } else {
                    0.0_f32.to_bits()
                }
            } else {
                0.0_f32.to_bits()
            };
            slot.store(pack_slot(generation, value_bits), Ordering::Relaxed);
        }
    }

    fn finish_page_write(&self, page: usize) {
        let page_state = &self.pages[page & 1];
        if page_state.sequence.load(Ordering::Relaxed) & 1 != 0 {
            page_state.sequence.fetch_add(1, Ordering::Release);
        }
    }

    fn invalidate(&self) -> u32 {
        self.display_token.store(0, Ordering::Release);
        self.last_update_micros.store(0, Ordering::Release);
        self.begin_next_generation()
    }

    /// Mark the source unavailable so stale data disappears immediately.
    pub(crate) fn mark_unavailable(&self) {
        self.invalidate();
    }

    pub(crate) fn set_live_mode(&self, enabled: bool) {
        self.live_mode.store(enabled, Ordering::Release);
    }

    pub(crate) fn live_mode(&self) -> bool {
        self.live_mode.load(Ordering::Acquire)
    }

    /// Atomically make one fully initialized page visible to the GUI.
    fn publish_page(&self, page: usize, generation: u32) {
        self.finish_page_write(page);
        self.display_token
            .store(pack_display_token(page, generation), Ordering::Release);
        self.last_update_micros
            .store(monotonic_micros().max(1), Ordering::Release);
    }

    fn publish_slot(&self, page: usize, generation: u32, bin: usize, value: f32) {
        let Some(slot) = self.pages[page & 1].slots.get(bin) else {
            return;
        };
        slot.store(
            pack_slot(generation, value.clamp(0.0, 1.0).to_bits()),
            Ordering::Relaxed,
        );
    }

    fn finish_block(&self) {
        self.last_update_micros
            .store(monotonic_micros().max(1), Ordering::Release);
    }

    /// Return a fresh non-silent snapshot, or the stable empty state.
    pub(crate) fn snapshot(&self) -> Option<IncomingWaveformSnapshot> {
        for _ in 0..SNAPSHOT_READ_RETRIES {
            let last_update = self.last_update_micros.load(Ordering::Acquire);
            let now = monotonic_micros().max(1);
            if last_update == 0 || now.saturating_sub(last_update) > SNAPSHOT_STALE_AFTER_MICROS {
                return None;
            }

            let token_before = self.display_token.load(Ordering::Acquire);
            let (page, generation) = unpack_display_token(token_before)?;
            let page_state = &self.pages[page];
            let sequence_before = page_state.sequence.load(Ordering::Acquire);
            if sequence_before & 1 != 0 {
                continue;
            }

            let mut snapshot = [0.0; INCOMING_WAVEFORM_BIN_COUNT];
            for (index, value) in snapshot.iter_mut().enumerate() {
                let slot = page_state.slots[index].load(Ordering::Relaxed);
                if slot_generation(slot) == generation {
                    let sample = f32::from_bits(slot_value_bits(slot));
                    *value = if sample.is_finite() {
                        sample.clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                }
            }

            let sequence_after = page_state.sequence.load(Ordering::Acquire);
            let token_after = self.display_token.load(Ordering::Acquire);
            if sequence_before != sequence_after
                || sequence_after & 1 != 0
                || token_before != token_after
            {
                continue;
            }

            let any_signal = snapshot.iter().any(|value| *value > SILENCE_FLOOR);
            return any_signal.then_some(snapshot);
        }
        None
    }

    #[cfg(all(test, feature = "vst3"))]
    pub(crate) fn set_last_update_micros_for_test(&self, value: u64) {
        self.last_update_micros.store(value, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RemapCaptureState {
    #[default]
    Normal,
    AwaitingRemappedBoundary,
    CapturingRemappedCycle,
}

/// Audio-owned peak aggregator that holds the last completed cycle for the GUI.
#[derive(Default)]
pub(crate) struct IncomingWaveformWriter {
    generation: u32,
    write_page: usize,
    page_open: bool,
    initial_snapshot_done: bool,
    last_raw_cycle_phase: Option<f32>,
    last_phase: Option<f32>,
    last_cycle_mapping: Option<[u32; 3]>,
    remap_state: RemapCaptureState,
    live_publication_suppressed: bool,
    current_bin: Option<usize>,
    current_peak: f32,
    cycle_has_signal: bool,
    block_has_signal: bool,
    live_mode: bool,
}

impl IncomingWaveformWriter {
    pub(crate) fn begin_block(&mut self, buffer: &IncomingWaveformBuffer) {
        let generation = buffer.generation();
        if self.generation != generation {
            // A buffer generation can change without the processor resetting its writer
            // (for example, while a VST3 runtime handoff is unavailable). Preserve the
            // established hidden-capture state for that internal handoff. An explicit
            // reset after mark_unavailable starts a fresh initial snapshot instead.
            let initial_snapshot_done = self.initial_snapshot_done || self.generation != 0;
            let live_publication_suppressed = self.live_publication_suppressed;
            if self.page_open {
                buffer.finish_page_write(self.write_page);
            }
            self.reset();
            self.initial_snapshot_done = initial_snapshot_done;
            self.live_publication_suppressed = live_publication_suppressed;
            self.generation = generation;
            self.write_page = buffer.writable_page(generation as usize);
            buffer.begin_page_write(self.write_page, generation, false);
            self.page_open = true;
        }

        let live_mode = buffer.live_mode();
        if !self.page_open || buffer.displayed_page() == Some(self.write_page) {
            self.rotate_from_display(buffer, live_mode);
        }

        if live_mode && !self.live_mode && self.page_open {
            buffer.begin_page_write(self.write_page, self.generation, true);
        }
        self.live_mode = live_mode;
        self.block_has_signal = false;
    }

    fn rotate_from_display(&mut self, buffer: &IncomingWaveformBuffer, seed_from_display: bool) {
        if self.page_open {
            buffer.finish_page_write(self.write_page);
            self.page_open = false;
        }
        let generation = buffer.begin_next_generation();
        let page = buffer.writable_page(self.write_page);
        buffer.begin_page_write(page, generation, seed_from_display);
        self.generation = generation;
        self.write_page = page;
        self.page_open = true;
    }

    fn open_next_capture(&mut self, buffer: &IncomingWaveformBuffer, seed_from_display: bool) {
        if self.page_open {
            buffer.finish_page_write(self.write_page);
            self.page_open = false;
        }
        let generation = buffer.begin_next_generation();
        let page = buffer.writable_page(self.write_page);
        buffer.begin_page_write(page, generation, seed_from_display);
        self.generation = generation;
        self.write_page = page;
        self.page_open = true;
    }

    fn open_after_invalidation(&mut self, buffer: &IncomingWaveformBuffer, generation: u32) {
        let page = buffer.writable_page(generation as usize);
        buffer.begin_page_write(page, generation, false);
        self.generation = generation;
        self.write_page = page;
        self.page_open = true;
    }

    fn clear_partial_capture(&mut self) {
        self.current_bin = None;
        self.current_peak = 0.0;
        self.cycle_has_signal = false;
        self.block_has_signal = false;
    }

    fn invalidate_capture(&mut self, buffer: &IncomingWaveformBuffer) {
        if self.page_open {
            buffer.finish_page_write(self.write_page);
            self.page_open = false;
        }
        let live_mode = self.live_mode;
        let generation = buffer.invalidate();
        self.initial_snapshot_done = true;
        self.live_publication_suppressed = true;
        self.clear_partial_capture();
        self.remap_state = RemapCaptureState::Normal;
        self.open_after_invalidation(buffer, generation);
        self.live_mode = live_mode;
    }

    fn start_remapped_capture(&mut self, buffer: &IncomingWaveformBuffer) {
        self.open_next_capture(buffer, false);
        self.clear_partial_capture();
        self.live_publication_suppressed = true;
    }

    fn complete_cycle(&mut self, buffer: &IncomingWaveformBuffer) {
        self.flush_current(buffer);
        if self.cycle_has_signal {
            buffer.publish_page(self.write_page, self.generation);
        } else {
            buffer.finish_page_write(self.write_page);
        }
        self.initial_snapshot_done = true;
        self.page_open = false;
        self.open_next_capture(buffer, self.live_mode);
        self.live_publication_suppressed = false;
        self.clear_partial_capture();
    }

    #[cfg(test)]
    pub(crate) fn record(
        &mut self,
        buffer: &IncomingWaveformBuffer,
        phase: f32,
        left: f32,
        right: f32,
    ) {
        self.record_with_cycle_mapping(buffer, phase, phase, 1.0, 0.0, 0.0, left, right);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_with_cycle_mapping(
        &mut self,
        buffer: &IncomingWaveformBuffer,
        raw_cycle_phase: f32,
        phase: f32,
        beats_per_cycle: f32,
        phase_offset: f32,
        swing: f32,
        left: f32,
        right: f32,
    ) {
        let raw_cycle_phase = raw_cycle_phase.rem_euclid(1.0);
        let phase = phase.rem_euclid(1.0);
        let cycle_mapping = [
            beats_per_cycle.to_bits(),
            phase_offset.to_bits(),
            swing.to_bits(),
        ];
        let cycle_mapping_changed = self
            .last_cycle_mapping
            .is_some_and(|previous| previous != cycle_mapping);
        let raw_cycle_complete = self.last_raw_cycle_phase.is_some_and(|previous| {
            previous > 0.5
                && raw_cycle_phase < 0.5
                && raw_cycle_phase + BACKWARD_PHASE_EPSILON < previous
        });
        let raw_discontinuity = self.last_raw_cycle_phase.is_some_and(|previous| {
            let moved_backward = raw_cycle_phase + BACKWARD_PHASE_EPSILON < previous;
            let moved_forward = raw_cycle_phase - previous > FORWARD_PHASE_DISCONTINUITY;
            (moved_backward && !raw_cycle_complete) || moved_forward
        });
        let offset_only_mapping_change = self.last_cycle_mapping.is_some_and(|previous| {
            previous[0] == cycle_mapping[0]
                && previous[1] != cycle_mapping[1]
                && previous[2] == cycle_mapping[2]
                && !raw_discontinuity
        });
        let transformed_cycle_wrap = self.last_phase.is_some_and(|previous| {
            previous > 0.5 && phase < 0.5 && phase + BACKWARD_PHASE_EPSILON < previous
        });

        // The raw phase is the only seek/discontinuity witness. Mapping changes are
        // handled separately so a transformed offset jump cannot clear the display.
        if raw_discontinuity {
            self.invalidate_capture(buffer);
        } else if offset_only_mapping_change {
            self.start_remapped_capture(buffer);
            self.remap_state = RemapCaptureState::AwaitingRemappedBoundary;
        } else if cycle_mapping_changed {
            self.invalidate_capture(buffer);
        } else if transformed_cycle_wrap {
            match self.remap_state {
                RemapCaptureState::Normal => self.complete_cycle(buffer),
                RemapCaptureState::AwaitingRemappedBoundary => {
                    self.start_remapped_capture(buffer);
                    self.remap_state = RemapCaptureState::CapturingRemappedCycle;
                }
                RemapCaptureState::CapturingRemappedCycle => {
                    self.complete_cycle(buffer);
                    self.remap_state = RemapCaptureState::Normal;
                }
            }
        }

        self.last_raw_cycle_phase = Some(raw_cycle_phase);
        self.last_phase = Some(phase);
        self.last_cycle_mapping = Some(cycle_mapping);

        let bin = ((phase * INCOMING_WAVEFORM_BIN_COUNT as f32).floor() as usize)
            .min(INCOMING_WAVEFORM_BIN_COUNT - 1);
        if self.current_bin != Some(bin) {
            self.flush_current(buffer);
            self.current_bin = Some(bin);
            self.current_peak = 0.0;
        }
        let peak = left.abs().max(right.abs());
        if peak.is_finite() {
            let peak = peak.clamp(0.0, 1.0);
            self.block_has_signal |= peak > SILENCE_FLOOR;
            self.cycle_has_signal |= peak > SILENCE_FLOOR;
            self.current_peak = self.current_peak.max(peak);
        }
    }

    pub(crate) fn finish_block(&mut self, buffer: &IncomingWaveformBuffer) {
        if !self.block_has_signal {
            return;
        }
        self.flush_current(buffer);
        if (self.live_mode
            && !self.live_publication_suppressed
            && self.remap_state == RemapCaptureState::Normal)
            || (!self.initial_snapshot_done && buffer.displayed_page().is_none())
        {
            buffer.publish_page(self.write_page, self.generation);
            self.page_open = false;
            self.initial_snapshot_done = true;
        } else {
            buffer.finish_block();
        }
        self.block_has_signal = false;
    }

    pub(crate) fn reset(&mut self) {
        self.reset_capture_state();
        self.initial_snapshot_done = false;
        self.live_publication_suppressed = false;
    }

    fn reset_capture_state(&mut self) {
        self.generation = 0;
        self.write_page = 0;
        self.page_open = false;
        self.last_raw_cycle_phase = None;
        self.last_phase = None;
        self.last_cycle_mapping = None;
        self.remap_state = RemapCaptureState::Normal;
        self.current_bin = None;
        self.current_peak = 0.0;
        self.cycle_has_signal = false;
        self.block_has_signal = false;
        self.live_mode = false;
    }

    fn flush_current(&self, buffer: &IncomingWaveformBuffer) {
        if let Some(bin) = self.current_bin {
            buffer.publish_slot(self.write_page, self.generation, bin, self.current_peak);
        }
    }
}

/// One enabled processing-block capture target.
pub(crate) struct IncomingWaveformCapture<'a> {
    buffer: &'a IncomingWaveformBuffer,
    writer: &'a mut IncomingWaveformWriter,
}

impl<'a> IncomingWaveformCapture<'a> {
    pub(crate) fn new(
        buffer: &'a IncomingWaveformBuffer,
        writer: &'a mut IncomingWaveformWriter,
    ) -> Self {
        writer.begin_block(buffer);
        Self { buffer, writer }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &mut self,
        raw_cycle_phase: f32,
        phase: f32,
        beats_per_cycle: f32,
        phase_offset: f32,
        swing: f32,
        left: f32,
        right: f32,
    ) {
        self.writer.record_with_cycle_mapping(
            self.buffer,
            raw_cycle_phase,
            phase,
            beats_per_cycle,
            phase_offset,
            swing,
            left,
            right,
        );
    }

    pub(crate) fn finish(self) {
        self.writer.finish_block(self.buffer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::swing_warp_phase;
    use crate::test_alloc::assert_no_alloc;

    #[test]
    fn fresh_buffer_has_a_stable_empty_snapshot() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        assert!(buffer.snapshot().is_none());
    }

    #[test]
    fn default_capture_maps_normalized_phase_to_fixed_bins() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        for step in 25..=75 {
            let phase = step as f32 / 100.0;
            let peak = match step {
                25 => 0.75,
                75 => 0.5,
                _ => 0.0,
            };
            writer.record(&buffer, phase, peak, -peak);
        }
        for step in 76..100 {
            writer.record(&buffer, step as f32 / 100.0, 0.0, 0.0);
        }
        writer.record(&buffer, 0.0, 0.0, 0.0);
        writer.finish_block(&buffer);

        let snapshot = buffer.snapshot().expect("signal should be available");
        assert!((snapshot[INCOMING_WAVEFORM_BIN_COUNT / 4] - 0.75).abs() < 1.0e-6);
        assert!((snapshot[INCOMING_WAVEFORM_BIN_COUNT * 3 / 4] - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn unavailable_and_silent_input_do_not_retain_old_signal() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record(&buffer, 0.8, 1.0, 0.0);
        writer.record(&buffer, 0.1, 0.0, 0.0);
        writer.finish_block(&buffer);
        assert!(buffer.snapshot().is_some());

        buffer.mark_unavailable();
        assert!(buffer.snapshot().is_none());

        writer.begin_block(&buffer);
        writer.record(&buffer, 0.12, 0.0, 0.0);
        writer.finish_block(&buffer);
        assert!(buffer.snapshot().is_none());
    }

    #[test]
    fn unavailable_reset_allows_a_new_initial_snapshot() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record(&buffer, 0.25, 0.8, 0.0);
        writer.finish_block(&buffer);
        assert!(buffer.snapshot().is_some());

        buffer.mark_unavailable();
        writer.reset();

        writer.begin_block(&buffer);
        writer.record(&buffer, 0.75, 0.6, 0.0);
        writer.finish_block(&buffer);

        let snapshot = buffer
            .snapshot()
            .expect("the first reactivated block should publish");
        assert_eq!(
            snapshot[(0.75 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.6
        );
        assert_eq!(
            snapshot[(0.25 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.0
        );
    }

    #[test]
    fn generation_mismatch_without_reset_preserves_hidden_capture_lifecycle() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record(&buffer, 0.25, 0.8, 0.0);
        writer.finish_block(&buffer);
        assert!(buffer.snapshot().is_some());

        buffer.mark_unavailable();
        writer.begin_block(&buffer);
        writer.record(&buffer, 0.75, 0.6, 0.0);
        writer.finish_block(&buffer);

        assert!(
            buffer.snapshot().is_none(),
            "an internal generation handoff must not publish a partial initial page"
        );
    }

    #[test]
    fn initial_snapshot_and_live_capture_remain_coherent() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();

        writer.begin_block(&buffer);
        writer.record_with_cycle_mapping(&buffer, 0.25, 0.25, 1.0, 0.0, 0.0, 0.8, 0.0);
        writer.finish_block(&buffer);
        let initial = buffer.snapshot().expect("the initial block should publish");
        assert_eq!(
            initial[(0.25 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.8
        );

        writer.begin_block(&buffer);
        writer.record_with_cycle_mapping(&buffer, 0.26, 0.75, 1.0, 0.0, 0.0, 0.4, 0.0);
        writer.finish_block(&buffer);
        let held = buffer
            .snapshot()
            .expect("a hidden non-live capture must retain the initial page");
        assert_eq!(
            held[(0.25 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.8
        );
        assert_eq!(
            held[(0.75 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.0
        );

        buffer.set_live_mode(true);
        writer.begin_block(&buffer);
        writer.record_with_cycle_mapping(&buffer, 0.27, 0.5, 1.0, 0.0, 0.0, 0.6, 0.0);
        writer.finish_block(&buffer);
        let live = buffer
            .snapshot()
            .expect("live capture should publish the seeded page");
        assert_eq!(
            live[(0.25 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.8
        );
        assert_eq!(
            live[(0.75 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.4
        );
        assert_eq!(
            live[(0.5 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.6
        );
    }

    #[test]
    fn cycle_wrap_holds_the_completed_cycle_while_capturing_the_next() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record(&buffer, 0.8, 0.9, 0.0);
        writer.record(&buffer, 0.1, 0.4, 0.0);
        for step in 11..=80 {
            writer.record(
                &buffer,
                step as f32 / 100.0,
                if step == 80 { 0.2 } else { 0.0 },
                0.0,
            );
        }
        writer.finish_block(&buffer);

        let snapshot = buffer.snapshot().expect("completed cycle should remain");
        assert!(
            (snapshot[(0.8 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize] - 0.9).abs() < 1.0e-6
        );
        assert_eq!(
            snapshot[(0.1 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.0
        );
    }

    #[test]
    fn live_mode_seeds_the_next_cycle_without_clearing_the_display() {
        let buffer = IncomingWaveformBuffer::default();
        buffer.set_live_mode(true);
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record(&buffer, 0.8, 0.9, 0.0);
        writer.record(&buffer, 0.1, 0.4, 0.0);
        writer.finish_block(&buffer);

        let snapshot = buffer
            .snapshot()
            .expect("live waveform should remain visible");
        assert!(
            (snapshot[(0.8 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize] - 0.9).abs() < 1.0e-6
        );
        assert!(
            (snapshot[(0.1 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize] - 0.4).abs() < 1.0e-6
        );
    }

    #[test]
    fn smaller_backward_seek_invalidates_later_phase_bins() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record(&buffer, 0.45, 0.9, 0.0);
        writer.record(&buffer, 0.2, 0.4, 0.0);
        writer.finish_block(&buffer);

        assert!(
            buffer.snapshot().is_none(),
            "a seek discards the incomplete capture"
        );
    }

    #[test]
    fn insignificant_backward_noise_does_not_restart_the_generation() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record(&buffer, 0.45, 0.9, 0.0);
        writer.record(&buffer, 0.45 - BACKWARD_PHASE_EPSILON * 0.5, 0.4, 0.0);
        writer.finish_block(&buffer);

        let snapshot = buffer.snapshot().expect("the first cycle is shown live");
        assert!(snapshot.iter().copied().fold(0.0_f32, f32::max) >= 0.9);
    }

    #[test]
    fn large_forward_seek_invalidates_earlier_phase_bins() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record(&buffer, 0.2, 0.8, 0.0);
        writer.record(&buffer, 0.6, 0.4, 0.0);
        writer.finish_block(&buffer);

        assert!(
            buffer.snapshot().is_none(),
            "a seek discards the incomplete capture"
        );
    }

    #[test]
    fn ordinary_forward_progression_keeps_the_current_generation() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record(&buffer, 0.2, 0.8, 0.0);
        writer.record(&buffer, 0.21, 0.4, 0.0);
        writer.finish_block(&buffer);

        let snapshot = buffer.snapshot().expect("the first cycle is shown live");
        assert!(snapshot[(0.2 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize] >= 0.8);
        assert!(snapshot[(0.21 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize] >= 0.4);
    }

    #[test]
    fn cycle_mapping_changes_invalidate_bins_from_the_previous_mapping() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);

        writer.record_with_cycle_mapping(&buffer, 0.2, 0.2, 1.0, 0.0, 0.0, 0.9, 0.0);
        writer.record_with_cycle_mapping(&buffer, 0.2, 0.2, 8.0, 0.0, 0.0, 0.6, 0.0);
        writer.record_with_cycle_mapping(&buffer, 0.2, 0.2, 8.0, 0.25, 0.0, 0.4, 0.0);
        writer.finish_block(&buffer);

        assert!(
            buffer.snapshot().is_none(),
            "a mapping change discards the incomplete capture"
        );
    }

    #[test]
    fn phase_offset_remapping_retains_old_display_until_two_transformed_wraps() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record_with_cycle_mapping(&buffer, 0.8, 0.8, 1.0, 0.0, 0.0, 0.9, 0.0);
        writer.record_with_cycle_mapping(&buffer, 0.01, 0.1, 1.0, 0.0, 0.0, 0.0, 0.0);
        writer.finish_block(&buffer);

        let old_bin = (0.8 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize;
        assert_eq!(buffer.snapshot().unwrap()[old_bin], 0.9);

        for (raw, phase, phase_offset, peak) in [
            (0.03, 0.35, 0.25, 0.4),
            (0.05, 0.55, 0.45, 0.5),
            (0.07, 0.25, 0.15, 0.3),
            (0.09, 0.75, 0.65, 0.6),
        ] {
            writer.begin_block(&buffer);
            writer.record_with_cycle_mapping(
                &buffer,
                raw,
                phase,
                1.0,
                phase_offset,
                0.0,
                peak,
                0.0,
            );
            writer.finish_block(&buffer);
            assert_eq!(buffer.snapshot().unwrap()[old_bin], 0.9);
        }

        writer.begin_block(&buffer);
        for (raw, phase) in [
            (0.11, 0.78),
            (0.13, 0.81),
            (0.15, 0.84),
            (0.17, 0.87),
            (0.19, 0.90),
            (0.21, 0.96),
            (0.23, 0.99),
            (0.25, 0.02),
        ] {
            writer.record_with_cycle_mapping(&buffer, raw, phase, 1.0, 0.65, 0.0, 0.0, 0.0);
        }
        writer.finish_block(&buffer);
        assert_eq!(buffer.snapshot().unwrap()[old_bin], 0.9);

        writer.begin_block(&buffer);
        for (raw, phase, peak) in [
            (0.29, 0.08, 0.0),
            (0.31, 0.25, 0.0),
            (0.33, 0.50, 0.0),
            (0.35, 0.75, 0.6),
            (0.37, 0.99, 0.0),
            (0.39, 0.02, 0.0),
        ] {
            writer.record_with_cycle_mapping(&buffer, raw, phase, 1.0, 0.65, 0.0, peak, 0.0);
        }

        let replacement = buffer
            .snapshot()
            .expect("a stable remapped cycle should publish atomically");
        assert_eq!(
            replacement[(0.75 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.6
        );
    }

    #[test]
    fn phase_offset_remapping_does_not_mask_a_true_phase_discontinuity() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record_with_cycle_mapping(&buffer, 0.8, 0.8, 1.0, 0.0, 0.0, 0.9, 0.0);
        writer.record_with_cycle_mapping(&buffer, 0.1, 0.1, 1.0, 0.0, 0.0, 0.0, 0.0);
        writer.finish_block(&buffer);
        assert!(buffer.snapshot().is_some());

        writer.begin_block(&buffer);
        writer.record_with_cycle_mapping(&buffer, 0.2, 0.12, 1.0, 0.25, 0.0, 0.4, 0.0);
        writer.finish_block(&buffer);

        assert!(
            buffer.snapshot().is_none(),
            "an offset change must not preserve a waveform across a real phase seek"
        );
    }

    #[test]
    fn swing_mapping_change_invalidates_instead_of_mixing_generations() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record_with_cycle_mapping(&buffer, 0.8, 0.8, 1.0, 0.0, 0.0, 0.9, 0.0);
        writer.record_with_cycle_mapping(&buffer, 0.01, 0.01, 1.0, 0.0, 0.0, 0.0, 0.0);
        writer.finish_block(&buffer);
        assert!(buffer.snapshot().is_some());

        writer.begin_block(&buffer);
        writer.record_with_cycle_mapping(&buffer, 0.02, 0.2, 1.0, 0.0, 0.0, 0.4, 0.0);
        writer.record_with_cycle_mapping(
            &buffer,
            0.02,
            swing_warp_phase(0.2, 1.0),
            1.0,
            0.0,
            1.0,
            0.6,
            0.0,
        );
        writer.finish_block(&buffer);

        assert!(
            buffer.snapshot().is_none(),
            "a swing remap must not expose bins from mixed phase transforms"
        );
    }

    #[test]
    fn forced_page_reuse_is_not_visible_until_the_seqlock_closes() {
        let buffer = IncomingWaveformBuffer::default();
        let generation = buffer.generation();
        let page = buffer.writable_page(generation as usize);
        buffer.begin_page_write(page, generation, false);
        for index in 0..INCOMING_WAVEFORM_BIN_COUNT {
            buffer.publish_slot(page, generation, index, 0.25);
        }
        buffer.publish_page(page, generation);

        let old_snapshot = buffer.snapshot().expect("initial page should be readable");
        assert!(old_snapshot
            .iter()
            .all(|value| (*value - 0.25).abs() < 1.0e-6));

        let replacement_generation = buffer.begin_next_generation();
        let replacement_page = buffer.writable_page(page);
        buffer.begin_page_write(replacement_page, replacement_generation, false);
        for index in 0..INCOMING_WAVEFORM_BIN_COUNT / 2 {
            buffer.publish_slot(replacement_page, replacement_generation, index, 0.75);
        }
        assert!(buffer
            .snapshot()
            .expect("displayed page remains stable during hidden reuse")
            .iter()
            .all(|value| (*value - 0.25).abs() < 1.0e-6));

        // A forced early token publication is rejected because the page is still odd.
        buffer.display_token.store(
            pack_display_token(replacement_page, replacement_generation),
            Ordering::Release,
        );
        assert!(
            buffer.snapshot().is_none(),
            "an odd page cannot expose a partial replacement"
        );

        for index in INCOMING_WAVEFORM_BIN_COUNT / 2..INCOMING_WAVEFORM_BIN_COUNT {
            buffer.publish_slot(replacement_page, replacement_generation, index, 0.75);
        }
        buffer.publish_page(replacement_page, replacement_generation);
        let snapshot = buffer.snapshot().expect("complete page should be readable");
        assert!(snapshot.iter().all(|value| (*value - 0.75).abs() < 1.0e-6));
    }

    #[test]
    fn default_audio_capture_is_allocation_free() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        let _ = monotonic_micros();

        assert_no_alloc(|| {
            writer.begin_block(&buffer);
            for index in 0..512 {
                let phase = index as f32 / 512.0;
                writer.record(&buffer, phase, 0.5, -0.25);
            }
            writer.finish_block(&buffer);
        });
    }

    #[test]
    fn silent_block_does_not_refresh_a_previous_signal() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record(&buffer, 0.8, 0.8, 0.0);
        writer.record(&buffer, 0.1, 0.0, 0.0);
        writer.finish_block(&buffer);
        assert!(buffer.snapshot().is_some());

        buffer.last_update_micros.store(1, Ordering::Release);
        writer.begin_block(&buffer);
        writer.record(&buffer, 0.12, 0.0, 0.0);
        writer.finish_block(&buffer);

        assert_eq!(
            buffer.last_update_micros.load(Ordering::Acquire),
            1,
            "an all-silent block must not renew the previous signal's freshness"
        );
    }
}
