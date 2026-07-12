//! Shared sample-offset parameter scheduling for CLAP and VST3 processing.

use toybox::clack_plugin::utils::ClapId;
use toybox::dsp::TransportState;

use crate::dsp::{DspSettings, DspTelemetry, PumpEngine};
use crate::incoming_waveform::IncomingWaveformCapture;
use crate::params::{apply_param_event, PumpParams};

#[derive(Clone, Copy, Debug)]
struct ScheduledParamEvent {
    sample_offset: usize,
    sequence: usize,
    param_id: ClapId,
    plain_value: f32,
}

const DEFAULT_EVENT_CAPACITY: usize = 256;

/// Reusable per-processor storage for one block of parameter automation.
pub(crate) struct ParamEventSchedule {
    events: Vec<ScheduledParamEvent>,
    max_events: usize,
    next_event: usize,
    frame_count: usize,
    next_sequence: usize,
}

impl Default for ParamEventSchedule {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_EVENT_CAPACITY)
    }
}

impl ParamEventSchedule {
    /// Create reusable scheduling storage with the expected host-point capacity.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            events: Vec::with_capacity(capacity),
            max_events: capacity,
            next_event: 0,
            frame_count: 0,
            next_sequence: 0,
        }
    }

    /// Create bounded scheduling storage without panicking on allocation failure.
    pub(crate) fn try_with_capacity(
        capacity: usize,
    ) -> Result<Self, std::collections::TryReserveError> {
        let mut events = Vec::new();
        events.try_reserve_exact(capacity)?;
        Ok(Self {
            events,
            max_events: capacity,
            next_event: 0,
            frame_count: 0,
            next_sequence: 0,
        })
    }

    /// Clear the previous block and set the valid end boundary for this block.
    pub(crate) fn begin_block(&mut self, frame_count: usize) {
        self.events.clear();
        self.next_event = 0;
        self.frame_count = frame_count;
        self.next_sequence = 0;
    }

    /// Add one plain-valued parameter point, clamping its offset to the block.
    #[cfg(any(test, feature = "vst3"))]
    pub(crate) fn push(&mut self, sample_offset: i64, param_id: ClapId, plain_value: f32) {
        self.push_event(sample_offset, param_id, plain_value);
    }

    /// Add one point without growing beyond the preallocated event bound.
    pub(crate) fn push_bounded(
        &mut self,
        sample_offset: i64,
        param_id: ClapId,
        plain_value: f32,
    ) -> bool {
        if self.events.len() >= self.max_events {
            return false;
        }
        self.push_event(sample_offset, param_id, plain_value);
        true
    }

    fn push_event(&mut self, sample_offset: i64, param_id: ClapId, plain_value: f32) {
        let sample_offset = sample_offset.clamp(0, self.frame_count as i64) as usize;
        self.events.push(ScheduledParamEvent {
            sample_offset,
            sequence: self.next_sequence,
            param_id,
            plain_value,
        });
        self.next_sequence = self.next_sequence.saturating_add(1);
    }

    /// Sort points chronologically while retaining source order at equal offsets.
    pub(crate) fn prepare(&mut self) {
        self.events
            .sort_unstable_by_key(|event| (event.sample_offset, event.sequence));
        self.next_event = 0;
    }

    /// Apply every point scheduled at or before `sample_offset`.
    pub(crate) fn apply_through(
        &mut self,
        sample_offset: usize,
        params: &PumpParams,
        settings: &mut DspSettings,
    ) {
        while let Some(event) = self.events.get(self.next_event).copied() {
            if event.sample_offset > sample_offset {
                break;
            }
            apply_param_event(params, event.param_id, event.plain_value);
            *settings = dsp_settings_from_params(params);
            self.next_event += 1;
        }
    }

    /// Apply all remaining points, including points clamped to the end boundary.
    pub(crate) fn apply_remaining(&mut self, params: &PumpParams, settings: &mut DspSettings) {
        self.apply_through(self.frame_count, params, settings);
    }

    #[cfg(test)]
    pub(crate) fn event_storage(&self) -> (usize, usize, usize) {
        (self.events.len(), self.events.capacity(), self.max_events)
    }
}

/// Snapshot the four host-automatable controls into DSP settings.
pub(crate) fn dsp_settings_from_params(params: &PumpParams) -> DspSettings {
    DspSettings {
        mix: params.mix(),
        phase_offset: params.phase_offset(),
        output_gain_db: params.output_gain_db(),
        beats_per_cycle: params.sync_beats_per_cycle(),
    }
}

/// Process one stereo block while applying parameter points at sample boundaries.
pub(crate) struct StereoBlockSlices<'a> {
    pub(crate) left: &'a mut [f32],
    pub(crate) right: &'a mut [f32],
}

pub(crate) fn process_stereo_block(
    engine: &mut PumpEngine,
    block: StereoBlockSlices<'_>,
    params: &PumpParams,
    schedule: &mut ParamEventSchedule,
    settings: &mut DspSettings,
    transport: TransportState,
    mut waveform: Option<IncomingWaveformCapture<'_>>,
) -> Option<DspTelemetry> {
    let StereoBlockSlices { left, right } = block;
    let frame_count = left.len().min(right.len());
    let mut transport_for_sample = transport;
    let mut last_telemetry = None;

    for sample_offset in 0..frame_count {
        schedule.apply_through(sample_offset, params, settings);
        let input_left = left[sample_offset];
        let input_right = right[sample_offset];
        let telemetry = engine.process_sample(
            &mut left[sample_offset],
            &mut right[sample_offset],
            *settings,
            transport_for_sample,
        );
        if let Some(capture) = waveform.as_mut() {
            capture.record(
                telemetry.phase,
                settings.beats_per_cycle,
                settings.phase_offset,
                input_left,
                input_right,
            );
        }
        last_telemetry = Some(telemetry);
        transport_for_sample.song_pos_beats = None;
    }

    schedule.apply_remaining(params, settings);
    if frame_count > 0 {
        if let Some(capture) = waveform {
            capture.finish();
        }
    }
    last_telemetry
}

/// Process one stereo block through raw host pointers that may be in-place.
///
/// # Safety
///
/// Every pointer must be valid for `frame_count` samples. Each output channel
/// must be writable, and the two channel ranges must not overlap each other.
/// An input channel may alias its corresponding output channel.
#[cfg(feature = "vst3")]
pub(crate) struct RawStereoBlock {
    pub(crate) num_samples: usize,
    pub(crate) input_left: *const f32,
    pub(crate) input_right: *const f32,
    pub(crate) output_left: *mut f32,
    pub(crate) output_right: *mut f32,
}

#[cfg(feature = "vst3")]
impl RawStereoBlock {
    /// Write silence without creating host-buffer slices that could alias.
    pub(crate) unsafe fn silence(&self) {
        for sample_offset in 0..self.num_samples {
            unsafe {
                self.output_left.add(sample_offset).write(0.0);
                self.output_right.add(sample_offset).write(0.0);
            }
        }
    }
}

#[cfg(feature = "vst3")]
pub(crate) unsafe fn process_stereo_block_raw(
    engine: &mut PumpEngine,
    block: RawStereoBlock,
    params: &PumpParams,
    schedule: &mut ParamEventSchedule,
    settings: &mut DspSettings,
    transport: TransportState,
    mut waveform: Option<IncomingWaveformCapture<'_>>,
) -> Option<DspTelemetry> {
    let mut transport_for_sample = transport;
    let mut last_telemetry = None;

    for sample_offset in 0..block.num_samples {
        schedule.apply_through(sample_offset, params, settings);
        // Read both inputs before either output is written so exact in-place
        // buffers preserve the original stereo sample pair.
        let mut left = unsafe { block.input_left.add(sample_offset).read() };
        let mut right = unsafe { block.input_right.add(sample_offset).read() };
        let input_left = left;
        let input_right = right;
        let telemetry =
            engine.process_sample(&mut left, &mut right, *settings, transport_for_sample);
        if let Some(capture) = waveform.as_mut() {
            capture.record(
                telemetry.phase,
                settings.beats_per_cycle,
                settings.phase_offset,
                input_left,
                input_right,
            );
        }
        last_telemetry = Some(telemetry);
        unsafe {
            block.output_left.add(sample_offset).write(left);
            block.output_right.add(sample_offset).write(right);
        }
        transport_for_sample.song_pos_beats = None;
    }

    schedule.apply_remaining(params, settings);
    if block.num_samples > 0 {
        if let Some(capture) = waveform {
            capture.finish();
        }
    }
    last_telemetry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::CURVE_TABLE_LEN;
    use crate::params::{
        PARAM_MIX_ID, PARAM_OUTPUT_GAIN_ID, PARAM_PHASE_OFFSET_ID, PARAM_SYNC_DIVISION_ID,
    };

    fn constant_curve(value: f32) -> [f32; CURVE_TABLE_LEN] {
        [value; CURVE_TABLE_LEN]
    }

    fn ramp_curve() -> [f32; CURVE_TABLE_LEN] {
        std::array::from_fn(|index| index as f32 / (CURVE_TABLE_LEN - 1) as f32)
    }

    fn playing_transport(song_pos_beats: f64) -> TransportState {
        TransportState {
            tempo_bpm: 60.0,
            is_playing: true,
            song_pos_beats: Some(song_pos_beats),
        }
    }

    #[test]
    fn continuous_points_start_smoothing_at_their_exact_samples() {
        let params = PumpParams::new();
        params.set_mix(1.0);
        let mut engine = PumpEngine::new(100.0, constant_curve(0.0));
        let mut schedule = ParamEventSchedule::default();
        schedule.begin_block(8);
        schedule.push(5, PARAM_MIX_ID, 1.0);
        schedule.push(2, PARAM_MIX_ID, 0.0);
        schedule.prepare();
        let mut settings = dsp_settings_from_params(&params);
        let mut left = [1.0; 8];
        let mut right = [1.0; 8];

        process_stereo_block(
            &mut engine,
            StereoBlockSlices {
                left: &mut left,
                right: &mut right,
            },
            &params,
            &mut schedule,
            &mut settings,
            playing_transport(0.0),
            None,
        );

        assert_eq!(&left[..2], &[0.0, 0.0]);
        assert!(left[2] > 0.0, "offset-2 target must affect sample 2");
        assert!(left[3] > left[2]);
        assert!(left[4] > left[3]);
        assert!(
            left[5] < left[4],
            "offset-5 target must reverse smoothing at sample 5"
        );
        assert_eq!(left, right);
        assert!((params.mix() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn stepped_division_changes_on_the_requested_boundary() {
        let params = PumpParams::new();
        params.set_mix(1.0);
        params.set_sync_division(4.0); // 1 beat
        let mut engine = PumpEngine::new(100.0, ramp_curve());
        let mut schedule = ParamEventSchedule::default();
        schedule.begin_block(6);
        schedule.push(3, PARAM_SYNC_DIVISION_ID, 5.0); // 2 beats
        schedule.prepare();
        let mut settings = dsp_settings_from_params(&params);
        let mut left = [1.0; 6];
        let mut right = [1.0; 6];

        process_stereo_block(
            &mut engine,
            StereoBlockSlices {
                left: &mut left,
                right: &mut right,
            },
            &params,
            &mut schedule,
            &mut settings,
            playing_transport(0.4),
            None,
        );

        for (actual, expected) in left.iter().zip([0.4, 0.41, 0.42, 0.215, 0.22, 0.225]) {
            assert!((actual - expected).abs() < 1.0e-4, "{actual} != {expected}");
        }
        assert_eq!(left, right);
        assert_eq!(params.sync_division(), 5);
    }

    #[test]
    fn unsorted_and_out_of_range_points_are_clamped_and_stably_ordered() {
        let params = PumpParams::new();
        let mut schedule = ParamEventSchedule::default();
        schedule.begin_block(4);
        schedule.push(99, PARAM_OUTPUT_GAIN_ID, 8.0);
        schedule.push(-12, PARAM_MIX_ID, 0.25);
        schedule.push(2, PARAM_MIX_ID, 0.5);
        schedule.push(2, PARAM_MIX_ID, 0.75);
        schedule.prepare();
        let mut settings = dsp_settings_from_params(&params);

        schedule.apply_through(0, &params, &mut settings);
        assert!((params.mix() - 0.25).abs() < f32::EPSILON);
        schedule.apply_through(1, &params, &mut settings);
        assert!((params.mix() - 0.25).abs() < f32::EPSILON);
        schedule.apply_through(2, &params, &mut settings);
        assert!((params.mix() - 0.75).abs() < f32::EPSILON);
        assert!((params.output_gain_db() - 0.0).abs() < f32::EPSILON);
        schedule.apply_remaining(&params, &mut settings);
        assert!((params.output_gain_db() - 8.0).abs() < f32::EPSILON);
    }

    #[test]
    fn all_advertised_parameters_update_settings_at_their_boundaries() {
        let params = PumpParams::new();
        let mut schedule = ParamEventSchedule::default();
        schedule.begin_block(5);
        schedule.push(1, PARAM_MIX_ID, 0.25);
        schedule.push(2, PARAM_PHASE_OFFSET_ID, 0.5);
        schedule.push(3, PARAM_OUTPUT_GAIN_ID, 6.0);
        schedule.push(4, PARAM_SYNC_DIVISION_ID, 6.0);
        schedule.prepare();
        let mut settings = dsp_settings_from_params(&params);

        schedule.apply_through(0, &params, &mut settings);
        assert!((settings.mix - 1.0).abs() < f32::EPSILON);
        schedule.apply_through(1, &params, &mut settings);
        assert!((settings.mix - 0.25).abs() < f32::EPSILON);
        assert!((settings.phase_offset - 0.0).abs() < f32::EPSILON);
        schedule.apply_through(2, &params, &mut settings);
        assert!((settings.phase_offset - 0.5).abs() < f32::EPSILON);
        assert!((settings.output_gain_db - 0.0).abs() < f32::EPSILON);
        schedule.apply_through(3, &params, &mut settings);
        assert!((settings.output_gain_db - 6.0).abs() < f32::EPSILON);
        assert!((settings.beats_per_cycle - 1.0).abs() < f32::EPSILON);
        schedule.apply_through(4, &params, &mut settings);
        assert!((settings.beats_per_cycle - 4.0).abs() < f32::EPSILON);
    }
}
