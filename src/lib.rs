//! Pump: freehand beat-synced gain shaping for sidechain-style ducking.

#![warn(missing_docs)]

use std::sync::atomic::Ordering;
use std::sync::Arc;

use toybox::clack_extensions::audio_ports::*;
use toybox::clack_extensions::gui::{PluginGui, PluginGuiImpl};
use toybox::clack_extensions::params::*;
use toybox::clack_extensions::state::{PluginState, PluginStateImpl};
use toybox::clack_plugin;
use toybox::clack_plugin::prelude::*;
use toybox::clack_plugin::stream::{InputStream, OutputStream};
use toybox::clap::automation::{AutomationEvent, AutomationQueue};
use toybox::clap::gui::host_param_requester;
use toybox::clap::prelude::apply_param_events;
use toybox::clap::process::{min_len, split_channel};
use toybox::clap::state::{read_versioned_payload, write_versioned_payload};
use toybox::clap::transport::transport_state_from_transport;
use toybox::dsp::AtomicF32;
use toybox::dsp::TransportState;

use crate::dsp::{DspSettings, PumpEngine};
use crate::gui::PumpGui;
use crate::params::{
    apply_param_event, decode_state_payload, encode_state_payload, get_param_value, param_count,
    text_to_value, value_to_text, write_param_info, PumpParams,
};

mod curve;
mod dsp;
mod gui;
mod params;
#[cfg(feature = "vst3")]
mod vst3;

/// Versioned state payload magic (`PUMP`).
const STATE_MAGIC: u32 = u32::from_le_bytes(*b"PUMP");
/// Versioned state payload format version.
const STATE_VERSION: u32 = 1;

/// CLAP plugin type for Pump.
pub struct PumpPlugin;

impl Plugin for PumpPlugin {
    type AudioProcessor<'a> = PumpAudioProcessor<'a>;
    type Shared<'a> = PumpShared;
    type MainThread<'a> = PumpMainThread<'a>;

    fn declare_extensions(builder: &mut PluginExtensions<Self>, _shared: Option<&PumpShared>) {
        builder
            .register::<PluginAudioPorts>()
            .register::<PluginParams>()
            .register::<PluginState>()
            .register::<PluginGui>();
    }
}

impl DefaultPluginFactory for PumpPlugin {
    fn get_descriptor() -> PluginDescriptor {
        use clack_plugin::plugin::features::*;

        PluginDescriptor::new("com.portalsurfer.pump", "pump")
            .with_vendor("portalsurfer")
            .with_features([AUDIO_EFFECT, STEREO])
            .with_description("Freehand beat-synced gain ducking effect")
    }

    fn new_shared(_host: HostSharedHandle<'_>) -> Result<Self::Shared<'_>, PluginError> {
        Ok(PumpShared {
            params: Arc::new(PumpParams::new()),
            status: Arc::new(GuiStatus::default()),
            automation_queue: Arc::new(AutomationQueue::default()),
        })
    }

    fn new_main_thread<'a>(
        host: HostMainThreadHandle<'a>,
        shared: &'a Self::Shared<'a>,
    ) -> Result<Self::MainThread<'a>, PluginError> {
        Ok(PumpMainThread {
            shared,
            gui: PumpGui::default(),
            host: host.shared(),
            automation_drain: Vec::new(),
        })
    }
}

/// Shared thread-safe plugin resources.
pub struct PumpShared {
    /// Shared parameter and curve state.
    pub params: Arc<PumpParams>,
    /// Real-time telemetry exposed to GUI.
    pub status: Arc<GuiStatus>,
    /// Pending GUI automation events destined for the host.
    pub automation_queue: Arc<AutomationQueue>,
}

impl PluginShared<'_> for PumpShared {}

/// Main-thread state for parameters, GUI, and state I/O.
pub struct PumpMainThread<'a> {
    /// Shared plugin resources.
    shared: &'a PumpShared,
    /// Host-parented editor wrapper.
    gui: PumpGui,
    /// Host shared handle used for flush requests.
    host: HostSharedHandle<'a>,
    /// Scratch vector for draining queued automation events.
    automation_drain: Vec<AutomationEvent>,
}

impl<'a> PluginMainThread<'a, PumpShared> for PumpMainThread<'a> {}

impl PluginAudioPortsImpl for PumpMainThread<'_> {
    fn count(&mut self, _is_input: bool) -> u32 {
        1
    }

    fn get(&mut self, index: u32, _is_input: bool, writer: &mut AudioPortInfoWriter) {
        if index != 0 {
            return;
        }
        writer.set(&AudioPortInfo {
            id: ClapId::new(0),
            name: b"main",
            channel_count: 2,
            flags: AudioPortFlags::IS_MAIN,
            port_type: Some(AudioPortType::STEREO),
            in_place_pair: None,
        });
    }
}

impl PluginMainThreadParams for PumpMainThread<'_> {
    fn count(&mut self) -> u32 {
        param_count()
    }

    fn get_info(&mut self, param_index: u32, info: &mut ParamInfoWriter) {
        write_param_info(param_index, info);
    }

    fn get_value(&mut self, param_id: ClapId) -> Option<f64> {
        get_param_value(self.shared.params.as_ref(), param_id)
    }

    fn value_to_text(
        &mut self,
        param_id: ClapId,
        value: f64,
        writer: &mut ParamDisplayWriter,
    ) -> std::fmt::Result {
        value_to_text(self.shared.params.as_ref(), param_id, value, writer)
    }

    fn text_to_value(&mut self, param_id: ClapId, text: &std::ffi::CStr) -> Option<f64> {
        text_to_value(param_id, text)
    }

    fn flush(
        &mut self,
        input_parameter_changes: &InputEvents,
        output_parameter_changes: &mut OutputEvents,
    ) {
        apply_param_events(input_parameter_changes, |param_id, value| {
            apply_param_event(self.shared.params.as_ref(), param_id, value as f32)
        });

        let _ = self
            .shared
            .automation_queue
            .drain_to_output(output_parameter_changes, &mut self.automation_drain);
    }
}

impl PluginStateImpl for PumpMainThread<'_> {
    fn save(&mut self, output: &mut OutputStream) -> Result<(), PluginError> {
        let payload = encode_state_payload(self.shared.params.as_ref());
        write_versioned_payload(output, STATE_MAGIC, STATE_VERSION, &payload)?;
        Ok(())
    }

    fn load(&mut self, input: &mut InputStream) -> Result<(), PluginError> {
        let payload = read_versioned_payload(input, STATE_MAGIC, &[STATE_VERSION])?;
        decode_state_payload(self.shared.params.as_ref(), &payload.payload)
            .map_err(PluginError::Message)
    }
}

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
            self.process_stereo_pair(left_pair, right_pair, settings, transport);
        }

        let _ = self
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

        let _ = self
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

        self.shared.status.update(last_phase, last_gain);

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

/// Shared GUI telemetry values updated by the audio thread.
#[derive(Default)]
pub struct GuiStatus {
    phase: AtomicF32,
    gain: AtomicF32,
}

impl GuiStatus {
    /// Update telemetry from the latest processed frame.
    pub fn update(&self, phase: f32, gain: f32) {
        self.phase.store(phase, Ordering::Relaxed);
        self.gain.store(gain, Ordering::Relaxed);
    }

    /// Read latest phase value.
    pub fn phase(&self) -> f32 {
        self.phase.load(Ordering::Relaxed).rem_euclid(1.0)
    }

    /// Read latest linear gain value.
    pub fn gain(&self) -> f32 {
        self.gain.load(Ordering::Relaxed).max(0.0)
    }
}

#[allow(clippy::question_mark)]
impl<'a> PluginGuiImpl for PumpMainThread<'a> {
    toybox::patchbay_clap_gui_callbacks!(
        gui = gui,
        preferred_size = crate::gui::preferred_window_size,
        show = |plugin: &mut Self| {
            plugin.gui.open(
                &plugin.shared.params,
                &plugin.shared.status,
                plugin.shared.automation_queue.clone(),
                host_param_requester(plugin.host),
            )
        }
    );
}

toybox::clap_plugin_entry!(PumpPlugin);

#[cfg(test)]
mod tests {
    use crate::dsp::db_to_linear;

    #[test]
    fn db_to_linear_matches_unity_at_zero_db() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1.0e-6);
    }
}
