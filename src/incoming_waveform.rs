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
    enabled: AtomicBool,
    generation: AtomicU32,
    values: [AtomicU32; INCOMING_WAVEFORM_BIN_COUNT],
    value_generations: [AtomicU32; INCOMING_WAVEFORM_BIN_COUNT],
    last_update_micros: AtomicU64,
}

impl Default for IncomingWaveformBuffer {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            generation: AtomicU32::new(1),
            values: array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits())),
            value_generations: array::from_fn(|_| AtomicU32::new(0)),
            last_update_micros: AtomicU64::new(0),
        }
    }
}

impl IncomingWaveformBuffer {
    /// Enable or disable capture, invalidating every previously published bin.
    pub(crate) fn set_enabled(&self, enabled: bool) {
        if self.enabled.swap(enabled, Ordering::AcqRel) != enabled {
            self.invalidate();
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

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

    fn invalidate(&self) {
        self.begin_next_generation();
        self.last_update_micros.store(0, Ordering::Release);
    }

    /// Mark the source unavailable so stale data disappears immediately.
    pub(crate) fn mark_unavailable(&self) {
        if self.is_enabled() {
            self.invalidate();
        }
    }

    fn publish(&self, generation: u32, bin: usize, value: f32) {
        let Some(slot) = self.values.get(bin) else {
            return;
        };
        slot.store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        self.value_generations[bin].store(generation, Ordering::Release);
    }

    fn finish_block(&self) {
        self.last_update_micros
            .store(monotonic_micros().max(1), Ordering::Release);
    }

    /// Return a fresh non-silent snapshot, or the stable empty state.
    pub(crate) fn snapshot(&self) -> Option<IncomingWaveformSnapshot> {
        if !self.is_enabled() {
            return None;
        }
        let last_update = self.last_update_micros.load(Ordering::Acquire);
        let now = monotonic_micros().max(1);
        if last_update == 0 || now.saturating_sub(last_update) > SNAPSHOT_STALE_AFTER_MICROS {
            return None;
        }

        let generation = self.generation();
        let mut any_signal = false;
        let snapshot = array::from_fn(|index| {
            if self.value_generations[index].load(Ordering::Acquire) != generation {
                return 0.0;
            }
            let value = f32::from_bits(self.values[index].load(Ordering::Relaxed));
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

/// Audio-owned peak aggregator that publishes at most once per visited bin.
#[derive(Default)]
pub(crate) struct IncomingWaveformWriter {
    generation: u32,
    last_phase: Option<f32>,
    last_cycle_mapping: Option<[u32; 2]>,
    current_bin: Option<usize>,
    current_peak: f32,
    block_has_signal: bool,
}

impl IncomingWaveformWriter {
    pub(crate) fn begin_block(&mut self, buffer: &IncomingWaveformBuffer) -> bool {
        if !buffer.is_enabled() {
            self.reset();
            return false;
        }
        let generation = buffer.generation();
        if self.generation != generation {
            self.reset();
            self.generation = generation;
        }
        self.block_has_signal = false;
        true
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
        if cycle_mapping_changed || phase_discontinuity {
            self.generation = buffer.begin_next_generation();
            self.current_bin = None;
            self.current_peak = 0.0;
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
        self.block_has_signal = false;
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
    ) -> Option<Self> {
        writer
            .begin_block(buffer)
            .then_some(Self { buffer, writer })
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
    fn disabled_capture_has_a_stable_empty_snapshot() {
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        assert!(!writer.begin_block(&buffer));
        assert!(buffer.snapshot().is_none());
    }

    #[test]
    fn enabled_capture_maps_normalized_phase_to_fixed_bins() {
        let buffer = IncomingWaveformBuffer::default();
        buffer.set_enabled(true);
        let mut writer = IncomingWaveformWriter::default();
        assert!(writer.begin_block(&buffer));
        for step in 25..=75 {
            let phase = step as f32 / 100.0;
            let peak = match step {
                25 => 0.75,
                75 => 0.5,
                _ => 0.0,
            };
            writer.record(&buffer, phase, peak, -peak);
        }
        writer.finish_block(&buffer);

        let snapshot = buffer.snapshot().expect("signal should be available");
        assert!((snapshot[INCOMING_WAVEFORM_BIN_COUNT / 4] - 0.75).abs() < 1.0e-6);
        assert!((snapshot[INCOMING_WAVEFORM_BIN_COUNT * 3 / 4] - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn unavailable_and_silent_input_do_not_retain_old_signal() {
        let buffer = IncomingWaveformBuffer::default();
        buffer.set_enabled(true);
        let mut writer = IncomingWaveformWriter::default();
        assert!(writer.begin_block(&buffer));
        writer.record(&buffer, 0.2, 1.0, 0.0);
        writer.finish_block(&buffer);
        assert!(buffer.snapshot().is_some());

        buffer.mark_unavailable();
        assert!(buffer.snapshot().is_none());

        assert!(writer.begin_block(&buffer));
        writer.record(&buffer, 0.3, 0.0, 0.0);
        writer.finish_block(&buffer);
        assert!(buffer.snapshot().is_none());
    }

    #[test]
    fn cycle_wrap_invalidates_unvisited_bins_from_the_previous_cycle() {
        let buffer = IncomingWaveformBuffer::default();
        buffer.set_enabled(true);
        let mut writer = IncomingWaveformWriter::default();
        assert!(writer.begin_block(&buffer));
        writer.record(&buffer, 0.8, 0.9, 0.0);
        writer.record(&buffer, 0.1, 0.4, 0.0);
        writer.finish_block(&buffer);

        let snapshot = buffer.snapshot().expect("new cycle signal should remain");
        assert_eq!(
            snapshot[(0.8 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.0
        );
        assert!(
            (snapshot[(0.1 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize] - 0.4).abs() < 1.0e-6
        );
    }

    #[test]
    fn smaller_backward_seek_invalidates_later_phase_bins() {
        let buffer = IncomingWaveformBuffer::default();
        buffer.set_enabled(true);
        let mut writer = IncomingWaveformWriter::default();
        assert!(writer.begin_block(&buffer));
        writer.record(&buffer, 0.45, 0.9, 0.0);
        writer.record(&buffer, 0.2, 0.4, 0.0);
        writer.finish_block(&buffer);

        let snapshot = buffer.snapshot().expect("post-seek signal should remain");
        assert_eq!(
            snapshot[(0.45 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.0,
            "bins after a sub-half-cycle backward seek must not remain visible"
        );
        assert!(
            (snapshot[(0.2 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize] - 0.4).abs() < 1.0e-6
        );
    }

    #[test]
    fn insignificant_backward_noise_does_not_restart_the_generation() {
        let buffer = IncomingWaveformBuffer::default();
        buffer.set_enabled(true);
        let mut writer = IncomingWaveformWriter::default();
        assert!(writer.begin_block(&buffer));
        writer.record(&buffer, 0.45, 0.9, 0.0);
        writer.record(&buffer, 0.45 - BACKWARD_PHASE_EPSILON * 0.5, 0.4, 0.0);
        writer.finish_block(&buffer);

        let snapshot = buffer.snapshot().expect("signal should remain available");
        assert!(snapshot.iter().copied().fold(0.0_f32, f32::max) >= 0.9);
    }

    #[test]
    fn large_forward_seek_invalidates_earlier_phase_bins() {
        let buffer = IncomingWaveformBuffer::default();
        buffer.set_enabled(true);
        let mut writer = IncomingWaveformWriter::default();
        assert!(writer.begin_block(&buffer));
        writer.record(&buffer, 0.2, 0.8, 0.0);
        writer.record(&buffer, 0.6, 0.4, 0.0);
        writer.finish_block(&buffer);

        let snapshot = buffer.snapshot().expect("post-seek signal should remain");
        assert_eq!(
            snapshot[(0.2 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize],
            0.0,
            "bins before a large forward seek must not remain visible"
        );
        assert!(
            (snapshot[(0.6 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize] - 0.4).abs() < 1.0e-6
        );
    }

    #[test]
    fn ordinary_forward_progression_keeps_the_current_generation() {
        let buffer = IncomingWaveformBuffer::default();
        buffer.set_enabled(true);
        let mut writer = IncomingWaveformWriter::default();
        assert!(writer.begin_block(&buffer));
        writer.record(&buffer, 0.2, 0.8, 0.0);
        writer.record(&buffer, 0.21, 0.4, 0.0);
        writer.finish_block(&buffer);

        let snapshot = buffer.snapshot().expect("signal should remain available");
        assert!(snapshot[(0.2 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize] >= 0.8);
        assert!(snapshot[(0.21 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize] >= 0.4);
    }

    #[test]
    fn cycle_mapping_changes_invalidate_bins_from_the_previous_mapping() {
        let buffer = IncomingWaveformBuffer::default();
        buffer.set_enabled(true);
        let mut writer = IncomingWaveformWriter::default();
        assert!(writer.begin_block(&buffer));

        writer.record_with_cycle_mapping(&buffer, 0.2, 1.0, 0.0, 0.9, 0.0);
        writer.record_with_cycle_mapping(&buffer, 0.2, 8.0, 0.0, 0.6, 0.0);
        writer.record_with_cycle_mapping(&buffer, 0.2, 8.0, 0.25, 0.4, 0.0);
        writer.finish_block(&buffer);

        let snapshot = buffer
            .snapshot()
            .expect("signal from the latest cycle mapping should remain");
        assert!(
            (snapshot[(0.2 * INCOMING_WAVEFORM_BIN_COUNT as f32) as usize] - 0.4).abs() < 1.0e-6,
            "only the signal recorded after the final mapping change may remain"
        );
        assert_eq!(snapshot.iter().filter(|peak| **peak > 0.0).count(), 1);
    }

    #[test]
    fn disabling_invalidates_data_and_reenabling_starts_empty() {
        let buffer = IncomingWaveformBuffer::default();
        buffer.set_enabled(true);
        let mut writer = IncomingWaveformWriter::default();
        assert!(writer.begin_block(&buffer));
        writer.record(&buffer, 0.5, 1.0, 0.0);
        writer.finish_block(&buffer);
        assert!(buffer.snapshot().is_some());

        buffer.set_enabled(false);
        assert!(buffer.snapshot().is_none());
        buffer.set_enabled(true);
        assert!(buffer.snapshot().is_none());
    }

    #[test]
    fn enabled_audio_capture_is_allocation_free() {
        let buffer = IncomingWaveformBuffer::default();
        buffer.set_enabled(true);
        let mut writer = IncomingWaveformWriter::default();
        let _ = monotonic_micros();

        assert_no_alloc(|| {
            assert!(writer.begin_block(&buffer));
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
        buffer.set_enabled(true);
        let mut writer = IncomingWaveformWriter::default();
        assert!(writer.begin_block(&buffer));
        writer.record(&buffer, 0.2, 0.8, 0.0);
        writer.finish_block(&buffer);
        assert!(buffer.snapshot().is_some());

        buffer.last_update_micros.store(1, Ordering::Release);
        assert!(writer.begin_block(&buffer));
        writer.record(&buffer, 0.3, 0.0, 0.0);
        writer.finish_block(&buffer);

        assert_eq!(
            buffer.last_update_micros.load(Ordering::Acquire),
            1,
            "an all-silent block must not renew the previous signal's freshness"
        );
    }
}
