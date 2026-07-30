//! Lock-free, cycle-aligned incoming-audio visualization snapshots.

use std::array;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::time_utils::monotonic_micros;

/// Fixed horizontal resolution shared by capture and both editor renderers.
pub(crate) const INCOMING_WAVEFORM_BIN_COUNT: usize = 96;
const SNAPSHOT_STALE_AFTER_MICROS: u64 = 750_000;
const SILENCE_FLOOR: f32 = 1.0e-4;
const BACKWARD_PHASE_EPSILON: f32 = 1.0e-5;
const FORWARD_PHASE_DISCONTINUITY: f32 = 0.05;

/// A bounded GUI snapshot of peak input amplitude over one normalized cycle.
pub(crate) type IncomingWaveformSnapshot = [f32; INCOMING_WAVEFORM_BIN_COUNT];

/// Shared atomics written by the audio thread and sampled by the GUI thread.
pub(crate) struct IncomingWaveformBuffer {
    generation: AtomicU32,
    displayed_generation: AtomicU32,
    live_mode: AtomicBool,
    values: [[AtomicU32; INCOMING_WAVEFORM_BIN_COUNT]; 2],
    value_generations: [[AtomicU32; INCOMING_WAVEFORM_BIN_COUNT]; 2],
    last_update_micros: AtomicU64,
}

impl Default for IncomingWaveformBuffer {
    fn default() -> Self {
        Self {
            generation: AtomicU32::new(1),
            // Show the initial capture live. Once that first cycle completes, subsequent
            // captures remain hidden until their completed generation swaps in atomically.
            displayed_generation: AtomicU32::new(1),
            live_mode: AtomicBool::new(false),
            values: array::from_fn(|_| array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits()))),
            value_generations: array::from_fn(|_| array::from_fn(|_| AtomicU32::new(0))),
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

    fn begin_next_generation_seeded_from_display(&self) -> u32 {
        let generation = self.begin_next_generation();
        self.seed_generation_from_display(generation);
        generation
    }

    fn seed_generation_from_display(&self, generation: u32) {
        let displayed = self.displayed_generation.load(Ordering::Acquire);
        if displayed == 0 || displayed == generation {
            return;
        }
        let source_page = displayed as usize & 1;
        let target_page = generation as usize & 1;
        for index in 0..INCOMING_WAVEFORM_BIN_COUNT {
            if self.value_generations[target_page][index].load(Ordering::Acquire) == generation {
                continue;
            }
            let value = if self.value_generations[source_page][index].load(Ordering::Acquire)
                == displayed
            {
                self.values[source_page][index].load(Ordering::Relaxed)
            } else {
                0.0_f32.to_bits()
            };
            self.values[target_page][index].store(value, Ordering::Relaxed);
            self.value_generations[target_page][index].store(generation, Ordering::Release);
        }
    }

    fn invalidate(&self) -> u32 {
        self.displayed_generation.store(0, Ordering::Release);
        self.begin_next_generation();
        self.last_update_micros.store(0, Ordering::Release);
        self.generation()
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

    /// Atomically make one fully captured cycle visible to the GUI.
    fn publish_completed_generation(&self, generation: u32) {
        self.displayed_generation
            .store(generation, Ordering::Release);
        self.last_update_micros
            .store(monotonic_micros().max(1), Ordering::Release);
    }

    fn publish(&self, generation: u32, bin: usize, value: f32) {
        let page = generation as usize & 1;
        let Some(slot) = self.values[page].get(bin) else {
            return;
        };
        slot.store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        self.value_generations[page][bin].store(generation, Ordering::Release);
    }

    fn finish_block(&self) {
        self.last_update_micros
            .store(monotonic_micros().max(1), Ordering::Release);
    }

    /// Return a fresh non-silent snapshot, or the stable empty state.
    pub(crate) fn snapshot(&self) -> Option<IncomingWaveformSnapshot> {
        let last_update = self.last_update_micros.load(Ordering::Acquire);
        let now = monotonic_micros().max(1);
        if last_update == 0 || now.saturating_sub(last_update) > SNAPSHOT_STALE_AFTER_MICROS {
            return None;
        }

        let generation = self.displayed_generation.load(Ordering::Acquire);
        if generation == 0 {
            return None;
        }
        let page = generation as usize & 1;
        let mut any_signal = false;
        let snapshot = array::from_fn(|index| {
            if self.value_generations[page][index].load(Ordering::Acquire) != generation {
                return 0.0;
            }
            let value = f32::from_bits(self.values[page][index].load(Ordering::Relaxed));
            let value = if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            };
            any_signal |= value > SILENCE_FLOOR;
            value
        });
        any_signal.then_some(snapshot)
    }

    #[cfg(all(test, feature = "vst3"))]
    pub(crate) fn set_last_update_micros_for_test(&self, value: u64) {
        self.last_update_micros.store(value, Ordering::Release);
    }
}

/// Audio-owned peak aggregator that holds the last completed cycle for the GUI.
#[derive(Default)]
pub(crate) struct IncomingWaveformWriter {
    generation: u32,
    last_phase: Option<f32>,
    last_cycle_mapping: Option<[u32; 2]>,
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
            self.reset();
            self.generation = generation;
        }
        let live_mode = buffer.live_mode();
        if live_mode && !self.live_mode {
            buffer.seed_generation_from_display(self.generation);
        }
        self.live_mode = live_mode;
        self.block_has_signal = false;
    }

    #[cfg(test)]
    pub(crate) fn record(
        &mut self,
        buffer: &IncomingWaveformBuffer,
        phase: f32,
        left: f32,
        right: f32,
    ) {
        self.record_with_cycle_mapping(buffer, phase, 1.0, 0.0, left, right);
    }

    pub(crate) fn record_with_cycle_mapping(
        &mut self,
        buffer: &IncomingWaveformBuffer,
        phase: f32,
        beats_per_cycle: f32,
        phase_offset: f32,
        left: f32,
        right: f32,
    ) {
        let phase = phase.rem_euclid(1.0);
        let cycle_mapping = [beats_per_cycle.to_bits(), phase_offset.to_bits()];
        let cycle_mapping_changed = self
            .last_cycle_mapping
            .is_some_and(|previous| previous != cycle_mapping);
        let phase_discontinuity = self.last_phase.is_some_and(|previous| {
            phase + BACKWARD_PHASE_EPSILON < previous
                || phase - previous > FORWARD_PHASE_DISCONTINUITY
        });
        let cycle_complete = self.last_phase.is_some_and(|previous| {
            previous > 0.5 && phase < 0.5 && phase + BACKWARD_PHASE_EPSILON < previous
        });
        if cycle_mapping_changed || phase_discontinuity {
            if cycle_complete && !cycle_mapping_changed {
                self.flush_current(buffer);
                if self.cycle_has_signal {
                    buffer.publish_completed_generation(self.generation);
                }
                self.generation = if self.live_mode {
                    let next = buffer.begin_next_generation_seeded_from_display();
                    buffer.publish_completed_generation(next);
                    next
                } else {
                    buffer.begin_next_generation()
                };
            } else {
                self.generation = buffer.invalidate();
            }
            self.current_bin = None;
            self.current_peak = 0.0;
            self.cycle_has_signal = false;
            self.block_has_signal = false;
        }
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
        buffer.finish_block();
        self.block_has_signal = false;
    }

    pub(crate) fn reset(&mut self) {
        self.generation = 0;
        self.last_phase = None;
        self.last_cycle_mapping = None;
        self.current_bin = None;
        self.current_peak = 0.0;
        self.cycle_has_signal = false;
        self.block_has_signal = false;
        self.live_mode = false;
    }

    fn flush_current(&self, buffer: &IncomingWaveformBuffer) {
        if let Some(bin) = self.current_bin {
            buffer.publish(self.generation, bin, self.current_peak);
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

    pub(crate) fn record(
        &mut self,
        phase: f32,
        beats_per_cycle: f32,
        phase_offset: f32,
        left: f32,
        right: f32,
    ) {
        self.writer.record_with_cycle_mapping(
            self.buffer,
            phase,
            beats_per_cycle,
            phase_offset,
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

        writer.record_with_cycle_mapping(&buffer, 0.2, 1.0, 0.0, 0.9, 0.0);
        writer.record_with_cycle_mapping(&buffer, 0.2, 8.0, 0.0, 0.6, 0.0);
        writer.record_with_cycle_mapping(&buffer, 0.2, 8.0, 0.25, 0.4, 0.0);
        writer.finish_block(&buffer);

        assert!(
            buffer.snapshot().is_none(),
            "a mapping change discards the incomplete capture"
        );
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
