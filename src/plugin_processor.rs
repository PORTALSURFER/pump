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
        })
    }

    fn process(
        &mut self,
        process: Process,
        mut audio: Audio,
        events: Events,
    ) -> Result<ProcessStatus, PluginError> {
        apply_param_events(events.input, |param_id, value| {
            apply_param_event(self.shared.params.as_ref(), param_id, value as f32)
        });

        let revision = self.shared.params.curve_revision();
        if revision != self.last_curve_revision {
            self.engine
                .set_target_curve(self.shared.params.curve_snapshot());
            self.last_curve_revision = revision;
        }

        let settings = DspSettings {
            mix: self.shared.params.mix(),
            depth: self.shared.params.depth(),
            phase_offset: self.shared.params.phase_offset(),
            output_gain_db: self.shared.params.output_gain_db(),
            beats_per_cycle: self.shared.params.sync_beats_per_cycle(),
        };

        let transport = transport_state_from_transport(process.transport.copied());
        let phase_running = transport.is_playing || transport.song_pos_beats.is_none();
        let gui_phase = gui_phase_from_transport(transport, settings, self.shared.status.phase());
        let beat_phase =
            host_beat_phase(transport).unwrap_or_else(|| self.shared.status.beat_phase());
        self.shared.status.update(
            gui_phase,
            self.shared.status.gain(),
            GuiTransportTelemetry {
                is_playing: phase_running,
                has_host_beats_timeline: transport.song_pos_beats.is_some(),
                beat_phase,
                tempo_bpm: transport.tempo_bpm,
                beats_per_cycle: settings.beats_per_cycle,
            },
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
            self.process_stereo_pair(left_pair, right_pair, settings, transport, phase_running);
        }

        let _stats = self
            .shared
            .automation_queue
            .drain_to_output(events.output, &mut self.automation_drain);

        Ok(ProcessStatus::Continue)
    }
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
        settings: DspSettings,
        transport: TransportState,
        phase_running: bool,
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

        let host_is_playing = phase_running;
        let host_has_beats_timeline = transport.song_pos_beats.is_some();
        let mut transport_for_sample = transport;
        let mut last_phase = 0.0_f32;
        let mut last_gain = 1.0_f32;

        for frame in 0..frames {
            let telemetry = self.engine.process_sample(
                &mut self.scratch_left[frame],
                &mut self.scratch_right[frame],
                settings,
                transport_for_sample,
            );
            transport_for_sample.song_pos_beats = None;
            last_phase = telemetry.phase;
            last_gain = telemetry.gain;
        }

        self.shared.status.update(
            last_phase,
            last_gain,
            GuiTransportTelemetry {
                is_playing: host_is_playing,
                has_host_beats_timeline: host_has_beats_timeline,
                beat_phase: host_beat_phase(transport)
                    .unwrap_or_else(|| self.shared.status.beat_phase()),
                tempo_bpm: transport.tempo_bpm,
                beats_per_cycle: settings.beats_per_cycle,
            },
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
