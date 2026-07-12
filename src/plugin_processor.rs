use super::*;
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
}

impl<'a> PluginAudioProcessor<'a, PumpShared, PumpMainThread<'a>> for PumpAudioProcessor<'a> {
    fn activate(
        _host: HostAudioProcessorHandle<'a>,
        _main_thread: &mut PumpMainThread<'a>,
        shared: &'a PumpShared,
        audio_config: PluginAudioConfiguration,
    ) -> Result<Self, PluginError> {
        let curve = shared.params.curve_snapshot();
        Ok(Self {
            shared,
            engine: PumpEngine::new(audio_config.sample_rate as f32, curve),
            last_curve_revision: shared.params.curve_revision(),
            scratch_left: Vec::new(),
            scratch_right: Vec::new(),
            automation_drain: Vec::new(),
            param_schedule: ParamEventSchedule::with_capacity(
                (audio_config.max_frames_count as usize).saturating_mul(4),
            ),
        })
    }

    fn process(
        &mut self,
        process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        let frame_count = audio.frames_count() as usize;
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

        let transport = transport_state_from_transport(process.transport.copied());
        let gui_phase = gui_phase_from_transport(transport, settings, self.shared.status.phase());
        self.shared.status.update(
            gui_phase,
            self.shared.status.gain(),
            gui_transport_telemetry(
                transport,
                settings.beats_per_cycle,
                self.shared.status.beat_phase(),
            ),
        );

        for mut port_pair in &mut audio {
            let Some(mut channels) = port_pair.channels()?.into_f32() else {
                continue;
            };
            let mut iter = channels.iter_mut();
            let Some(left_pair) = iter.next() else {
                continue;
            };
            let Some(right_pair) = iter.next() else {
                continue;
            };
            self.process_stereo_pair(left_pair, right_pair, &mut settings, transport);
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

fn collect_clap_param_points(
    input: &InputEvents<'_>,
    frame_count: usize,
    schedule: &mut ParamEventSchedule,
) {
    schedule.begin_block(frame_count);
    for event in input {
        if let Some(CoreEventSpace::ParamValue(param)) = event.as_core_event() {
            if let Some(param_id) = param.param_id() {
                schedule.push(
                    i64::from(event.header().time()),
                    param_id,
                    param.value() as f32,
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
            apply_param_event(self.shared.params.as_ref(), param_id, value as f32)
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
    ) {
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
            return;
        }

        self.ensure_scratch(frames);

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
            &mut self.scratch_left[..frames],
            &mut self.scratch_right[..frames],
            self.shared.params.as_ref(),
            &mut self.param_schedule,
            settings,
            transport,
        );
        let (last_phase, last_gain) = telemetry
            .map(|telemetry| (telemetry.phase, telemetry.gain))
            .unwrap_or((0.0, 1.0));

        self.shared.status.update(
            last_phase,
            last_gain,
            gui_transport_telemetry(
                transport,
                settings.beats_per_cycle,
                self.shared.status.beat_phase(),
            ),
        );

        if let Some(out_left) = left_output {
            out_left[..frames].copy_from_slice(&self.scratch_left[..frames]);
        }
        if let Some(out_right) = right_output {
            out_right[..frames].copy_from_slice(&self.scratch_right[..frames]);
        }
    }

    fn ensure_scratch(&mut self, frames: usize) {
        if self.scratch_left.len() < frames {
            self.scratch_left.resize(frames, 0.0);
        }
        if self.scratch_right.len() < frames {
            self.scratch_right.resize(frames, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::PARAM_MIX_ID;
    use toybox::clack_plugin::events::event_types::ParamValueEvent;
    use toybox::clack_plugin::events::Pckn;
    use toybox::clack_plugin::utils::Cookie;

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
}
