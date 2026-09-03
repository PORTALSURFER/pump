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

/// Snapshot the host-automatable controls into DSP settings.
pub(crate) fn dsp_settings_from_params(params: &PumpParams) -> DspSettings {
    DspSettings {
        mix: params.mix(),
        depth_db: params.depth_db(),
        floor_db: params.floor_db(),
        phase_offset: params.phase_offset(),
        output_gain_db: params.output_gain_db(),
        beats_per_cycle: params.sync_beats_per_cycle(),
        smooth: params.smooth(),
        swing: params.swing(),
        timing_mode: params.timing_mode(),
        free_rate_hz: params.free_rate_hz(),
        bypassed: params.bypassed(),
    }
}

/// Process one stereo block while applying parameter points at sample boundaries.
pub(crate) struct StereoBlockSlices<'a> {
    pub(crate) left: &'a mut [f32],
    pub(crate) right: &'a mut [f32],
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn process_stereo_block(
    engine: &mut PumpEngine,
    block: StereoBlockSlices<'_>,
    params: &PumpParams,
    schedule: &mut ParamEventSchedule,
    settings: &mut DspSettings,
    last_curve_revision: &mut u32,
    transport: TransportState,
    mut waveform: Option<IncomingWaveformCapture<'_>>,
) -> Option<DspTelemetry> {
    let StereoBlockSlices { left, right } = block;
    let frame_count = left.len().min(right.len());
    let mut transport_for_sample = transport;
    let mut last_telemetry = None;
    let mut minimum_reduction_gain = 1.0_f32;
    let mut input_active = false;

    for sample_offset in 0..frame_count {
        schedule.apply_through(sample_offset, params, settings);
        let revision = params.curve_revision();
        if revision != *last_curve_revision {
            engine.set_target_curve(params.curve_snapshot());
            *last_curve_revision = revision;
        }
        let input_left = left[sample_offset];
        let input_right = right[sample_offset];
        let (telemetry, raw_cycle_phase) = engine.process_sample_with_raw_cycle_phase(
            &mut left[sample_offset],
            &mut right[sample_offset],
            *settings,
            transport_for_sample,
        );
        if !telemetry.bypassed && input_left.abs().max(input_right.abs()) > 1.0e-5 {
            input_active = true;
            minimum_reduction_gain = minimum_reduction_gain.min(telemetry.reduction_gain);
        }
        if let Some(capture) = waveform.as_mut() {
            capture.record(
                raw_cycle_phase,
                telemetry.phase,
                settings.beats_per_cycle,
                settings.swing,
                settings.timing_mode,
                input_left,
                input_right,
            );
        }
        last_telemetry = Some(telemetry);
        transport_for_sample.song_pos_beats = None;
    }

    schedule.apply_remaining(params, settings);
    if frame_count == 0 {
        if let Some(capture) = waveform.as_mut() {
            capture.reconcile_cycle_mapping(
                settings.beats_per_cycle,
                settings.swing,
                settings.timing_mode,
            );
        }
    } else if let Some(capture) = waveform {
        capture.finish();
    }
    last_telemetry.map(|mut telemetry| {
        if telemetry.bypassed {
            telemetry.reduction_gain = 1.0;
            telemetry.input_active = false;
        } else {
            telemetry.reduction_gain = minimum_reduction_gain;
            telemetry.input_active = input_active;
        }
        telemetry
    })
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
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn process_stereo_block_raw(
    engine: &mut PumpEngine,
    block: RawStereoBlock,
    params: &PumpParams,
    schedule: &mut ParamEventSchedule,
    settings: &mut DspSettings,
    last_curve_revision: &mut u32,
    transport: TransportState,
    mut waveform: Option<IncomingWaveformCapture<'_>>,
) -> Option<DspTelemetry> {
    let mut transport_for_sample = transport;
    let mut last_telemetry = None;
    let mut minimum_reduction_gain = 1.0_f32;
    let mut input_active = false;

    for sample_offset in 0..block.num_samples {
        schedule.apply_through(sample_offset, params, settings);
        let revision = params.curve_revision();
        if revision != *last_curve_revision {
            engine.set_target_curve(params.curve_snapshot());
            *last_curve_revision = revision;
        }
        // Read both inputs before either output is written so exact in-place
        // buffers preserve the original stereo sample pair.
        let mut left = unsafe { block.input_left.add(sample_offset).read() };
        let mut right = unsafe { block.input_right.add(sample_offset).read() };
        let input_left = left;
        let input_right = right;
        let (telemetry, raw_cycle_phase) = engine.process_sample_with_raw_cycle_phase(
            &mut left,
            &mut right,
            *settings,
            transport_for_sample,
        );
        if !telemetry.bypassed && input_left.abs().max(input_right.abs()) > 1.0e-5 {
            input_active = true;
            minimum_reduction_gain = minimum_reduction_gain.min(telemetry.reduction_gain);
        }
        if let Some(capture) = waveform.as_mut() {
            capture.record(
                raw_cycle_phase,
                telemetry.phase,
                settings.beats_per_cycle,
                settings.swing,
                settings.timing_mode,
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
    if block.num_samples == 0 {
        if let Some(capture) = waveform.as_mut() {
            capture.reconcile_cycle_mapping(
                settings.beats_per_cycle,
                settings.swing,
                settings.timing_mode,
            );
        }
    } else if let Some(capture) = waveform {
        capture.finish();
    }
    last_telemetry.map(|mut telemetry| {
        if telemetry.bypassed {
            telemetry.reduction_gain = 1.0;
            telemetry.input_active = false;
        } else {
            telemetry.reduction_gain = minimum_reduction_gain;
            telemetry.input_active = input_active;
        }
        telemetry
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::CURVE_TABLE_LEN;
    #[cfg(feature = "vst3")]
    use crate::incoming_waveform::IncomingWaveformBuffer;
    use crate::incoming_waveform::IncomingWaveformWriter;
    use crate::params::{
        PARAM_BYPASS_ID, PARAM_DEPTH_ID, PARAM_FLOOR_ID, PARAM_FREE_RATE_ID, PARAM_MIX_ID,
        PARAM_OUTPUT_GAIN_ID, PARAM_PHASE_OFFSET_ID, PARAM_SMOOTH_ID, PARAM_SOUND_ID,
        PARAM_SWING_ID, PARAM_SYNC_DIVISION_ID, PARAM_TIMING_MODE_ID,
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

    fn params_with_distinct_sound_curves() -> PumpParams {
        let params = PumpParams::new();
        params.set_curve(&constant_curve(0.0));
        params.copy_active_to_inactive();
        params.set_active_sound(crate::params::SoundSide::B);
        params.set_curve(&constant_curve(1.0));
        params.set_active_sound(crate::params::SoundSide::A);
        params
    }

    #[test]
    fn sound_automation_retargets_at_zero_and_keeps_revision_owned_by_processor() {
        let params = params_with_distinct_sound_curves();
        let mut engine = PumpEngine::new(1_000.0, constant_curve(0.0));
        let mut schedule = ParamEventSchedule::default();
        schedule.begin_block(4);
        schedule.push(0, PARAM_SOUND_ID, 1.0);
        schedule.prepare();
        let mut settings = dsp_settings_from_params(&params);
        let mut last_curve_revision = params.curve_revision();
        let mut left = [1.0; 4];
        let mut right = [1.0; 4];

        process_stereo_block(
            &mut engine,
            StereoBlockSlices {
                left: &mut left,
                right: &mut right,
            },
            &params,
            &mut schedule,
            &mut settings,
            &mut last_curve_revision,
            playing_transport(0.0),
            None,
        );

        assert!(
            left[1] > left[0],
            "offset-zero sound target must affect the block: left={left:?}"
        );
        assert_eq!(last_curve_revision, params.curve_revision());

        schedule.begin_block(4);
        schedule.push(2, PARAM_SOUND_ID, 0.0);
        schedule.prepare();
        process_stereo_block(
            &mut engine,
            StereoBlockSlices {
                left: &mut left,
                right: &mut right,
            },
            &params,
            &mut schedule,
            &mut settings,
            &mut last_curve_revision,
            playing_transport(0.0),
            None,
        );
        assert!(
            left[1] > left[2],
            "interior sound target must retarget at its offset: left={left:?}"
        );
        assert_eq!(last_curve_revision, params.curve_revision());
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
        let mut last_curve_revision = params.curve_revision();
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
            &mut last_curve_revision,
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
    fn free_timing_automation_takes_effect_at_its_sample_offset() {
        let params = PumpParams::new();
        let mut engine = PumpEngine::new(1_000.0, ramp_curve());
        let mut schedule = ParamEventSchedule::default();
        schedule.begin_block(6);
        schedule.push(
            2,
            PARAM_TIMING_MODE_ID,
            crate::params::TIMING_MODE_FREE as f32,
        );
        schedule.push(2, PARAM_FREE_RATE_ID, 10.0);
        schedule.prepare();
        let mut settings = dsp_settings_from_params(&params);
        let mut last_curve_revision = params.curve_revision();
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
            &mut last_curve_revision,
            playing_transport(3.0),
            None,
        );

        assert_eq!(params.timing_mode(), crate::params::TIMING_MODE_FREE);
        assert!((params.free_rate_hz() - 10.0).abs() < f32::EPSILON);
        assert!((left[2] - left[3]).abs() > 1.0e-5);
        assert_eq!(left, right);
    }

    #[cfg(feature = "vst3")]
    #[test]
    fn zero_frame_offset_and_free_rate_automation_retain_waveform_generation() {
        let params = PumpParams::new();
        params.set_timing_mode(crate::params::TIMING_MODE_FREE as f32);
        let buffer = IncomingWaveformBuffer::default();
        let mut writer = IncomingWaveformWriter::default();
        writer.begin_block(&buffer);
        writer.record_with_cycle_mapping_and_timing_mode(
            &buffer,
            0.25,
            0.25,
            1.0,
            0.0,
            crate::params::TIMING_MODE_FREE,
            0.8,
            0.0,
        );
        writer.finish_block(&buffer);

        let generation_before = buffer.generation_for_test();
        buffer.set_last_update_micros_for_test(1234);
        let mut schedule = ParamEventSchedule::default();
        schedule.begin_block(0);
        schedule.push(0, PARAM_PHASE_OFFSET_ID, 0.25);
        schedule.push(0, PARAM_FREE_RATE_ID, 17.0);
        schedule.prepare();
        let mut settings = dsp_settings_from_params(&params);
        let mut engine = PumpEngine::new(1_000.0, constant_curve(0.0));
        let mut last_curve_revision = params.curve_revision();

        process_stereo_block(
            &mut engine,
            StereoBlockSlices {
                left: &mut [],
                right: &mut [],
            },
            &params,
            &mut schedule,
            &mut settings,
            &mut last_curve_revision,
            playing_transport(0.0),
            Some(IncomingWaveformCapture::new_for_zero_frame(
                &buffer,
                &mut writer,
            )),
        );

        assert!((params.phase_offset() - 0.25).abs() < f32::EPSILON);
        assert!((params.free_rate_hz() - 17.0).abs() < f32::EPSILON);
        assert_eq!(buffer.generation_for_test(), generation_before);
        assert_eq!(buffer.last_update_micros_for_test(), 1234);
    }

    #[test]
    fn block_meter_uses_strongest_reduction_only_when_input_is_active() {
        let params = PumpParams::new();
        params.set_mix(1.0);
        let mut engine = PumpEngine::new(48_000.0, constant_curve(0.25));
        let mut schedule = ParamEventSchedule::default();
        schedule.begin_block(4);
        let mut settings = dsp_settings_from_params(&params);
        let mut last_curve_revision = params.curve_revision();
        let mut left = [0.0, 1.0, 0.5, 0.0];
        let mut right = [0.0, 0.5, 1.0, 0.0];
        let telemetry = process_stereo_block(
            &mut engine,
            StereoBlockSlices {
                left: &mut left,
                right: &mut right,
            },
            &params,
            &mut schedule,
            &mut settings,
            &mut last_curve_revision,
            playing_transport(0.0),
            None,
        )
        .expect("non-empty block should return telemetry");
        assert!(telemetry.input_active);
        assert!((telemetry.reduction_gain - 0.25).abs() < 1.0e-6);

        let mut silent_left = [0.0; 4];
        let mut silent_right = [0.0; 4];
        schedule.begin_block(4);
        let telemetry = process_stereo_block(
            &mut engine,
            StereoBlockSlices {
                left: &mut silent_left,
                right: &mut silent_right,
            },
            &params,
            &mut schedule,
            &mut settings,
            &mut last_curve_revision,
            playing_transport(0.0),
            None,
        )
        .expect("silent non-empty block should return telemetry");
        assert!(!telemetry.input_active);
        assert_eq!(telemetry.reduction_gain, 1.0);
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
        let mut last_curve_revision = params.curve_revision();
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
            &mut last_curve_revision,
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
        schedule.begin_block(8);
        schedule.push(1, PARAM_MIX_ID, 0.25);
        schedule.push(2, PARAM_DEPTH_ID, 60.0);
        schedule.push(3, PARAM_FLOOR_ID, -12.0);
        schedule.push(4, PARAM_PHASE_OFFSET_ID, 0.5);
        schedule.push(5, PARAM_OUTPUT_GAIN_ID, 6.0);
        schedule.push(6, PARAM_SYNC_DIVISION_ID, 6.0);
        schedule.push(7, PARAM_SMOOTH_ID, 0.75);
        schedule.push(7, PARAM_SWING_ID, 0.8);
        schedule.push(7, PARAM_BYPASS_ID, 1.0);
        schedule.prepare();
        let mut settings = dsp_settings_from_params(&params);

        schedule.apply_through(0, &params, &mut settings);
        assert!((settings.mix - 1.0).abs() < f32::EPSILON);
        schedule.apply_through(1, &params, &mut settings);
        assert!((settings.mix - 0.25).abs() < f32::EPSILON);
        assert!((settings.phase_offset - 0.0).abs() < f32::EPSILON);
        schedule.apply_through(2, &params, &mut settings);
        assert!((settings.depth_db - 60.0).abs() < f32::EPSILON);
        assert!((settings.floor_db + 60.0).abs() < f32::EPSILON);
        schedule.apply_through(3, &params, &mut settings);
        assert!((settings.floor_db + 12.0).abs() < f32::EPSILON);
        schedule.apply_through(4, &params, &mut settings);
        assert!((settings.phase_offset - 0.5).abs() < f32::EPSILON);
        assert!((settings.output_gain_db - 0.0).abs() < f32::EPSILON);
        schedule.apply_through(5, &params, &mut settings);
        assert!((settings.output_gain_db - 6.0).abs() < f32::EPSILON);
        assert!((settings.beats_per_cycle - 1.0).abs() < f32::EPSILON);
        schedule.apply_through(6, &params, &mut settings);
        assert!((settings.beats_per_cycle - 4.0).abs() < f32::EPSILON);
        schedule.apply_through(7, &params, &mut settings);
        assert!((settings.smooth - 0.75).abs() < f32::EPSILON);
        assert!((settings.swing - 0.8).abs() < f32::EPSILON);
        assert!(settings.bypassed);
    }

    #[test]
    fn bypass_points_retarget_at_exact_offsets_and_block_end_starts_next_sample() {
        let params = PumpParams::new();
        let mut engine = PumpEngine::new(1_000.0, constant_curve(0.0));
        let mut schedule = ParamEventSchedule::default();
        schedule.begin_block(4);
        schedule.push(0, PARAM_BYPASS_ID, 1.0);
        schedule.push(2, PARAM_BYPASS_ID, 0.0);
        schedule.push(2, PARAM_BYPASS_ID, 1.0);
        schedule.push(4, PARAM_BYPASS_ID, 0.0);
        schedule.prepare();
        let mut settings = dsp_settings_from_params(&params);
        let mut last_curve_revision = params.curve_revision();
        let mut left = [1.0; 4];
        let mut right = [1.0; 4];

        process_stereo_block(
            &mut engine,
            StereoBlockSlices {
                left: &mut left,
                right: &mut right,
            },
            &params,
            &mut schedule,
            &mut settings,
            &mut last_curve_revision,
            playing_transport(0.0),
            None,
        );

        assert_eq!(left, [0.2, 0.4, 0.6, 0.8]);
        assert!(!settings.bypassed);
        assert!(!params.bypassed());

        schedule.begin_block(1);
        schedule.prepare();
        let mut next_left = [1.0];
        let mut next_right = [1.0];
        process_stereo_block(
            &mut engine,
            StereoBlockSlices {
                left: &mut next_left,
                right: &mut next_right,
            },
            &params,
            &mut schedule,
            &mut settings,
            &mut last_curve_revision,
            playing_transport(0.04),
            None,
        );
        assert!(
            (next_left[0] - 0.64).abs() < 1.0e-6,
            "block-end event must first affect the next processed sample"
        );
    }

    #[test]
    fn full_bypass_idles_meter_but_keeps_waveform_capture_running() {
        let params = PumpParams::new();
        params.set_bypass(1.0);
        let mut engine = PumpEngine::new_with_bypass(1_000.0, constant_curve(0.0), true);
        let mut schedule = ParamEventSchedule::default();
        schedule.begin_block(4);
        let mut settings = dsp_settings_from_params(&params);
        let mut last_curve_revision = params.curve_revision();
        let mut left = [0.5, 0.25, 0.75, 0.125];
        let original = left;
        let mut right = left;
        let status = crate::GuiStatus::default();
        let mut writer = IncomingWaveformWriter::default();

        let telemetry = process_stereo_block(
            &mut engine,
            StereoBlockSlices {
                left: &mut left,
                right: &mut right,
            },
            &params,
            &mut schedule,
            &mut settings,
            &mut last_curve_revision,
            playing_transport(0.0),
            Some(IncomingWaveformCapture::new(
                status.incoming_waveform_buffer(),
                &mut writer,
            )),
        )
        .expect("non-empty block should return telemetry");

        assert_eq!(left, original);
        assert_eq!(right, original);
        assert!(telemetry.bypassed);
        assert_eq!(telemetry.reduction_gain, 1.0);
        assert!(!telemetry.input_active);
        assert!(
            status.incoming_waveform_snapshot().is_some(),
            "full bypass must continue waveform capture"
        );
    }
}
