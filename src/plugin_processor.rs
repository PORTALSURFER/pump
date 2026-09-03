use super::*;
use crate::incoming_waveform::{IncomingWaveformCapture, IncomingWaveformWriter};

const CLAP_PARAM_EVENTS_PER_FRAME: usize = 4;
const CLAP_BUFFER_ALLOCATION_ERROR: &str = "Unable to preallocate CLAP realtime processing buffers";

/// Audio-thread processor for Pump.
pub struct PumpAudioProcessor<'a> {
    /// Shared plugin resources.
    shared: &'a PumpShared,
    /// Real-time gain envelope engine.
    engine: PumpEngine,
    /// Last observed curve revision.
    last_curve_revision: u32,
    /// Temporary left-channel storage for non-inplace paths.
    scratch_left: Vec<f32>,
    /// Temporary right-channel storage for non-inplace paths.
    scratch_right: Vec<f32>,
    /// Scratch vector for draining queued automation events.
    automation_drain: Vec<AutomationEvent>,
    /// Reused sample-offset parameter schedule for the current block.
    param_schedule: ParamEventSchedule,
    /// Bounded audio-owned peak aggregator for optional GUI visualization.
    waveform_writer: IncomingWaveformWriter,
    /// Activation sample rate used to convert CLAP input latency to beats.
    sample_rate: f64,
}

impl<'a> PluginAudioProcessor<'a, PumpShared, PumpMainThread<'a>> for PumpAudioProcessor<'a> {
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        _main_thread: &mut PumpMainThread<'a>,
        shared: &'a PumpShared,
        audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        Self::new(shared, audio_config)
    }

    fn process(
        &mut self,
        process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        let frame_count = audio.frames_count() as usize;
        if frame_count > self.scratch_left.len() {
            self.shared.status.mark_gain_reduction_inactive();
            self.shared
                .status
                .incoming_waveform_buffer()
                .mark_unavailable();
            self.waveform_writer.reset();
            apply_param_events(events.input, |param_id, value| {
                apply_clap_param_event(self.shared.params.as_ref(), param_id, value as f32)
            });
            silence_audio(&mut audio);
            let _stats = self
                .shared
                .automation_queue
                .drain_to_output(events.output, &mut self.automation_drain);
            return Ok(ProcessStatus::Continue);
        }

        collect_clap_param_points(events.input, frame_count, &mut self.param_schedule);

        let revision = self.shared.params.curve_revision();
        if revision != self.last_curve_revision {
            self.engine
                .set_target_curve(self.shared.params.curve_snapshot());
            self.last_curve_revision = revision;
        }

        let mut settings = dsp_settings_from_params(self.shared.params.as_ref());
        self.param_schedule
            .apply_through(0, self.shared.params.as_ref(), &mut settings);

        let input_latency_samples = audio
            .port_pair(0)
            .and_then(|port_pair| port_pair.latencies().0);
        let transport = crate::transport::compensate_input_presentation_latency(
            transport_state_from_transport(process.transport.copied()),
            input_latency_samples,
            self.sample_rate,
        );
        let gui_phase = gui_phase_from_transport(transport, settings, self.shared.status.phase());
        self.shared.status.update_transport(
            gui_phase,
            gui_transport_telemetry(transport, settings, self.shared.status.beat_phase()),
        );

        let (source_present, source_processed) = audio
            .port_pair(0)
            .and_then(|mut port_pair| {
                let mut channels = port_pair.channels().ok()?.into_f32()?;
                let mut iter = channels.iter_mut();
                let left_pair = iter.next()?;
                let right_pair = iter.next()?;
                let source_present =
                    channel_pair_has_input(&left_pair) && channel_pair_has_input(&right_pair);
                let source_processed =
                    self.process_stereo_pair(left_pair, right_pair, &mut settings, transport);
                Some((source_present, source_processed))
            })
            .unwrap_or((false, false));
        if should_mark_waveform_unavailable(frame_count, source_present, source_processed) {
            self.shared.status.mark_gain_reduction_inactive();
            self.shared
                .status
                .incoming_waveform_buffer()
                .mark_unavailable();
            self.waveform_writer.reset();
        }

        self.param_schedule
            .apply_remaining(self.shared.params.as_ref(), &mut settings);

        let _stats = self
            .shared
            .automation_queue
            .drain_to_output(events.output, &mut self.automation_drain);

        Ok(ProcessStatus::Continue)
    }
}

fn channel_pair_has_input(pair: &ChannelPair<'_, f32>) -> bool {
    !matches!(pair, ChannelPair::OutputOnly(_))
}

fn should_mark_waveform_unavailable(
    frame_count: usize,
    source_present: bool,
    source_processed: bool,
) -> bool {
    !source_present || (frame_count > 0 && !source_processed)
}

impl<'a> PumpAudioProcessor<'a> {
    fn new(
        shared: &'a PumpShared,
        audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        let curve = shared.params.curve_snapshot();
        let max_frames = audio_config.max_frames_count as usize;
        let scratch_left = try_zeroed_vec(max_frames)?;
        let scratch_right = try_zeroed_vec(max_frames)?;
        let mut automation_drain = Vec::new();
        automation_drain
            .try_reserve_exact(shared.automation_queue.config().max_events)
            .map_err(|_| PluginError::Message(CLAP_BUFFER_ALLOCATION_ERROR))?;
        let param_capacity = max_frames.saturating_mul(CLAP_PARAM_EVENTS_PER_FRAME);
        let param_schedule = ParamEventSchedule::try_with_capacity(param_capacity)
            .map_err(|_| PluginError::Message(CLAP_BUFFER_ALLOCATION_ERROR))?;

        Ok(Self {
            shared,
            engine: PumpEngine::new_with_bypass(
                audio_config.sample_rate as f32,
                curve,
                shared.params.bypassed(),
            ),
            last_curve_revision: shared.params.curve_revision(),
            scratch_left,
            scratch_right,
            automation_drain,
            param_schedule,
            waveform_writer: IncomingWaveformWriter::default(),
            sample_rate: audio_config.sample_rate,
        })
    }
}

fn try_zeroed_vec(len: usize) -> Result<Vec<f32>, PluginError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(len)
        .map_err(|_| PluginError::Message(CLAP_BUFFER_ALLOCATION_ERROR))?;
    values.resize(len, 0.0);
    Ok(values)
}

fn silence_audio(audio: &mut Audio<'_>) {
    for mut port_pair in audio {
        let Ok(channels) = port_pair.channels() else {
            continue;
        };
        let Some(mut channels) = channels.into_f32() else {
            continue;
        };
        for channel in channels.iter_mut() {
            silence_channel(channel);
        }
    }
}

fn silence_channel(channel: ChannelPair<'_, f32>) {
    match channel {
        ChannelPair::InputOnly(_) => {}
        ChannelPair::OutputOnly(output)
        | ChannelPair::InputOutput(_, output)
        | ChannelPair::InPlace(output) => output.fill(0.0),
    }
}

fn collect_clap_param_points(
    input: &InputEvents<'_>,
    frame_count: usize,
    schedule: &mut ParamEventSchedule,
) {
    schedule.begin_block(frame_count);
    for event in input {
        if let Some(CoreEventSpace::ParamValue(param)) = event.as_core_event() {
            if let Some(param_id) = param.param_id() {
                schedule.push_bounded(
                    i64::from(event.header().time()),
                    param_id,
                    plain_value_from_clap_value(param_id, param.value()) as f32,
                );
            }
        }
    }
    schedule.prepare();
}

impl PluginAudioProcessorParams for PumpAudioProcessor<'_> {
    fn flush(
        &mut self,
        input_parameter_changes: &InputEvents,
        output_parameter_changes: &mut OutputEvents,
    ) {
        apply_param_events(input_parameter_changes, |param_id, value| {
            apply_clap_param_event(self.shared.params.as_ref(), param_id, value as f32)
        });

        let _stats = self
            .shared
            .automation_queue
            .drain_to_output(output_parameter_changes, &mut self.automation_drain);
    }
}

impl PumpAudioProcessor<'_> {
    fn process_stereo_pair(
        &mut self,
        left_pair: ChannelPair<'_, f32>,
        right_pair: ChannelPair<'_, f32>,
        settings: &mut DspSettings,
        transport: TransportState,
    ) -> bool {
        let (left_input, left_output, left_in_place) = split_channel(left_pair);
        let (right_input, right_output, right_in_place) = split_channel(right_pair);

        let frames = min_len(&[
            left_input.as_ref().map(|buf| buf.len()),
            left_output.as_ref().map(|buf| buf.len()),
            right_input.as_ref().map(|buf| buf.len()),
            right_output.as_ref().map(|buf| buf.len()),
        ])
        .unwrap_or(0);

        if frames == 0 {
            return false;
        }

        if frames > self.scratch_left.len() || frames > self.scratch_right.len() {
            return false;
        }

        for frame in 0..frames {
            self.scratch_left[frame] = if left_in_place {
                left_output
                    .as_deref()
                    .and_then(|buf| buf.get(frame))
                    .copied()
                    .unwrap_or(0.0)
            } else {
                left_input
                    .as_ref()
                    .and_then(|buf| buf.get(frame))
                    .copied()
                    .unwrap_or(0.0)
            };

            self.scratch_right[frame] = if right_in_place {
                right_output
                    .as_deref()
                    .and_then(|buf| buf.get(frame))
                    .copied()
                    .unwrap_or(0.0)
            } else {
                right_input
                    .as_ref()
                    .and_then(|buf| buf.get(frame))
                    .copied()
                    .unwrap_or(0.0)
            };
        }

        let telemetry = process_stereo_block(
            &mut self.engine,
            crate::sample_automation::StereoBlockSlices {
                left: &mut self.scratch_left[..frames],
                right: &mut self.scratch_right[..frames],
            },
            self.shared.params.as_ref(),
            &mut self.param_schedule,
            settings,
            &mut self.last_curve_revision,
            transport,
            Some(IncomingWaveformCapture::new(
                self.shared.status.incoming_waveform_buffer(),
                &mut self.waveform_writer,
            )),
        );
        let last_phase = telemetry.map(|telemetry| telemetry.phase).unwrap_or(0.0);

        self.shared.status.update_transport(
            last_phase,
            gui_transport_telemetry(transport, *settings, self.shared.status.beat_phase()),
        );
        if let Some(telemetry) = telemetry {
            self.shared
                .status
                .publish_gain_reduction(telemetry.reduction_gain, telemetry.input_active);
        } else {
            self.shared.status.mark_gain_reduction_inactive();
        }

        if let Some(out_left) = left_output {
            out_left[..frames].copy_from_slice(&self.scratch_left[..frames]);
        }
        if let Some(out_right) = right_output {
            out_right[..frames].copy_from_slice(&self.scratch_right[..frames]);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{PARAM_BYPASS_ID, PARAM_MIX_ID};
    use crate::test_alloc::assert_no_alloc;
    use toybox::clack_plugin::events::event_types::ParamValueEvent;
    use toybox::clack_plugin::events::io::{OutputEventBuffer, TryPushError};
    use toybox::clack_plugin::events::Pckn;
    use toybox::clack_plugin::events::UnknownEvent;
    use toybox::clack_plugin::utils::Cookie;
    use toybox::clap::automation::{AutomationConfig, DEFAULT_AUTOMATION_QUEUE_MAX_EVENTS};

    fn shared() -> PumpShared {
        PumpShared {
            params: Arc::new(PumpParams::new()),
            status: Arc::new(GuiStatus::default()),
            automation_queue: Arc::new(PumpAutomationQueue::default()),
        }
    }

    fn processor(shared: &PumpShared, max_frames_count: u32) -> PumpAudioProcessor<'_> {
        PumpAudioProcessor::new(
            shared,
            PluginAudioConfiguration {
                sample_rate: 48_000.0,
                min_frames_count: 1,
                max_frames_count,
            },
        )
        .expect("processor buffers should preallocate")
    }

    #[derive(Default)]
    struct CountingOutput {
        count: usize,
    }

    impl OutputEventBuffer for CountingOutput {
        fn try_push(&mut self, _event: &UnknownEvent) -> Result<(), TryPushError> {
            self.count += 1;
            Ok(())
        }
    }

    #[test]
    fn clap_points_retain_offsets_and_are_scheduled_chronologically() {
        let events = [
            ParamValueEvent::new(7, PARAM_MIX_ID, Pckn::match_all(), 0.7, Cookie::empty()),
            ParamValueEvent::new(0, PARAM_MIX_ID, Pckn::match_all(), 0.2, Cookie::empty()),
            ParamValueEvent::new(3, PARAM_MIX_ID, Pckn::match_all(), 0.4, Cookie::empty()),
        ];
        let input = InputEvents::from_buffer(&events);
        let params = PumpParams::new();
        let mut schedule = ParamEventSchedule::default();
        collect_clap_param_points(&input, 5, &mut schedule);
        let mut settings = dsp_settings_from_params(&params);

        schedule.apply_through(0, &params, &mut settings);
        assert!((params.mix() - 0.2).abs() < f32::EPSILON);
        schedule.apply_through(2, &params, &mut settings);
        assert!((params.mix() - 0.2).abs() < f32::EPSILON);
        schedule.apply_through(3, &params, &mut settings);
        assert!((params.mix() - 0.4).abs() < f32::EPSILON);
        schedule.apply_remaining(&params, &mut settings);
        assert!((params.mix() - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn activation_preallocates_declared_scratch_and_bounded_event_storage() {
        let shared = shared();
        let processor = processor(&shared, 512);

        assert_eq!(processor.scratch_left.len(), 512);
        assert_eq!(processor.scratch_right.len(), 512);
        assert!(processor.automation_drain.is_empty());
        assert!(processor.automation_drain.capacity() >= DEFAULT_AUTOMATION_QUEUE_MAX_EVENTS);
        assert_eq!(
            processor.param_schedule.event_storage().2,
            512 * CLAP_PARAM_EVENTS_PER_FRAME
        );
    }

    #[test]
    fn first_maximum_block_with_separate_buffers_is_allocation_free() {
        const FRAMES: usize = 512;
        let shared = shared();
        shared.params.set_mix(0.0);
        let mut processor = processor(&shared, FRAMES as u32);
        processor.param_schedule.begin_block(FRAMES);
        let mut settings = dsp_settings_from_params(shared.params.as_ref());
        let left_input = [0.25; FRAMES];
        let right_input = [-0.5; FRAMES];
        let mut left_output = [0.0; FRAMES];
        let mut right_output = [0.0; FRAMES];

        assert_no_alloc(|| {
            processor.process_stereo_pair(
                ChannelPair::InputOutput(&left_input, &mut left_output),
                ChannelPair::InputOutput(&right_input, &mut right_output),
                &mut settings,
                TransportState::default(),
            );
        });

        assert!(left_output.iter().all(|sample| sample.is_finite()));
        assert!(right_output.iter().all(|sample| sample.is_finite()));
        for (left, right) in left_output.iter().zip(right_output) {
            assert!((right + 2.0 * left).abs() < 1.0e-6);
        }
    }

    #[test]
    fn maximum_block_with_sample_dense_bypass_retargets_without_allocations_or_locks() {
        const FRAMES: usize = 512;
        let shared = shared();
        let mut processor = processor(&shared, FRAMES as u32);
        processor.param_schedule.begin_block(FRAMES);
        for frame in 0..FRAMES {
            assert!(processor.param_schedule.push_bounded(
                frame as i64,
                PARAM_BYPASS_ID,
                (frame & 1) as f32,
            ));
        }
        processor.param_schedule.prepare();
        let mut settings = dsp_settings_from_params(shared.params.as_ref());
        let left_input = [0.25; FRAMES];
        let right_input = [-0.5; FRAMES];
        let mut left_output = [0.0; FRAMES];
        let mut right_output = [0.0; FRAMES];

        assert_no_alloc(|| {
            processor.process_stereo_pair(
                ChannelPair::InputOutput(&left_input, &mut left_output),
                ChannelPair::InputOutput(&right_input, &mut right_output),
                &mut settings,
                TransportState::default(),
            );
        });

        assert!(left_output.iter().all(|sample| sample.is_finite()));
        assert!(right_output.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn in_place_block_uses_preallocated_scratch() {
        const FRAMES: usize = 64;
        let shared = shared();
        shared.params.set_mix(0.0);
        let mut processor = processor(&shared, FRAMES as u32);
        processor.param_schedule.begin_block(FRAMES);
        let mut settings = dsp_settings_from_params(shared.params.as_ref());
        let mut left = [0.125; FRAMES];
        let mut right = [-0.25; FRAMES];

        processor.process_stereo_pair(
            ChannelPair::InPlace(&mut left),
            ChannelPair::InPlace(&mut right),
            &mut settings,
            TransportState::default(),
        );

        assert!(left.iter().all(|sample| sample.is_finite()));
        assert!(right.iter().all(|sample| sample.is_finite()));
        for (left, right) in left.iter().zip(right) {
            assert!((right + 2.0 * left).abs() < 1.0e-6);
        }
    }

    #[test]
    fn oversized_host_block_is_silenced_without_using_scratch() {
        let shared = shared();
        let processor = processor(&shared, 16);
        assert!(17 > processor.scratch_left.len());

        let mut left_output = [2.0; 17];
        let mut right_output = [3.0; 17];
        silence_channel(ChannelPair::InPlace(&mut left_output));
        silence_channel(ChannelPair::OutputOnly(&mut right_output));

        assert_eq!(left_output, [0.0; 17]);
        assert_eq!(right_output, [0.0; 17]);
    }

    #[test]
    fn default_waveform_capture_stays_allocation_free_in_clap_processing() {
        const FRAMES: usize = 128;
        let shared = shared();
        let mut processor = processor(&shared, FRAMES as u32);
        processor.param_schedule.begin_block(FRAMES);
        let mut settings = dsp_settings_from_params(shared.params.as_ref());
        let mut left = [0.75; FRAMES];
        let mut right = [-0.25; FRAMES];
        let _ = monotonic_micros();

        assert_no_alloc(|| {
            assert!(processor.process_stereo_pair(
                ChannelPair::InPlace(&mut left),
                ChannelPair::InPlace(&mut right),
                &mut settings,
                TransportState::default(),
            ));
        });

        let snapshot = shared
            .status
            .incoming_waveform_snapshot()
            .expect("default processing should publish the input envelope");
        assert!(snapshot.iter().copied().fold(0.0_f32, f32::max) >= 0.75);
    }

    #[test]
    fn zero_frame_clap_block_preserves_present_source_but_not_missing_source() {
        assert!(!should_mark_waveform_unavailable(0, true, false));
        assert!(should_mark_waveform_unavailable(0, false, false));
        assert!(should_mark_waveform_unavailable(64, true, false));
        assert!(!should_mark_waveform_unavailable(64, true, true));
    }

    #[test]
    fn output_only_clap_channels_do_not_count_as_input() {
        let input = [0.0; 1];
        let mut output = [0.0; 1];
        assert!(channel_pair_has_input(&ChannelPair::InputOnly(&input)));
        assert!(channel_pair_has_input(&ChannelPair::InputOutput(
            &input,
            &mut output
        )));
        assert!(channel_pair_has_input(&ChannelPair::InPlace(&mut output)));
        assert!(!channel_pair_has_input(&ChannelPair::OutputOnly(
            &mut output
        )));
    }

    #[test]
    fn clap_input_latency_propagates_to_the_block_transport() {
        let transport = TransportState {
            tempo_bpm: 120.0,
            song_pos_beats: Some(4.0),
            ..TransportState::default()
        };

        let compensated = crate::transport::compensate_input_presentation_latency(
            transport,
            Some(24_000),
            48_000.0,
        );

        assert!((compensated.song_pos_beats.unwrap_or_default() - 3.0).abs() < 1.0e-6);
    }

    #[test]
    fn realtime_flush_drains_a_full_automation_queue_without_allocating() {
        let shared = shared();
        let mut processor = processor(&shared, 64);
        let config = AutomationConfig::default();
        for index in 0..DEFAULT_AUTOMATION_QUEUE_MAX_EVENTS {
            let param_id = ClapId::new((index as u32).saturating_add(1));
            assert_eq!(
                shared.automation_queue.push_value(&config, param_id, 0.5),
                toybox::clap::automation::AutomationEnqueueStatus::Enqueued
            );
        }
        let input_events: [ParamValueEvent; 0] = [];
        let input = InputEvents::from_buffer(&input_events);
        let mut sink = CountingOutput::default();
        let mut output = OutputEvents::from_buffer(&mut sink);

        assert_no_alloc(|| processor.flush(&input, &mut output));

        assert_eq!(sink.count, DEFAULT_AUTOMATION_QUEUE_MAX_EVENTS);
        assert!(processor.automation_drain.is_empty());
    }

    #[test]
    fn dense_parameter_input_is_truncated_at_preallocated_capacity() {
        let params = PumpParams::new();
        let mut schedule = ParamEventSchedule::with_capacity(2);
        let events = [
            ParamValueEvent::new(0, PARAM_MIX_ID, Pckn::match_all(), 0.1, Cookie::empty()),
            ParamValueEvent::new(1, PARAM_MIX_ID, Pckn::match_all(), 0.2, Cookie::empty()),
            ParamValueEvent::new(2, PARAM_MIX_ID, Pckn::match_all(), 0.3, Cookie::empty()),
        ];
        let input = InputEvents::from_buffer(&events);

        assert_no_alloc(|| collect_clap_param_points(&input, 16, &mut schedule));

        assert_eq!(schedule.event_storage(), (2, 2, 2));

        schedule.prepare();
        let mut settings = dsp_settings_from_params(&params);
        schedule.apply_remaining(&params, &mut settings);
        assert!((params.mix() - 0.2).abs() < f32::EPSILON);
    }
}
