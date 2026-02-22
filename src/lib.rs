//! Pump: node-based beat-synced gain shaping for sidechain-style ducking.

#![warn(missing_docs)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
use toybox::dsp::{AtomicF32, TransportState};

use crate::dsp::{DspSettings, PumpEngine};
use crate::gui::PumpGui;
use crate::params::{
    apply_param_event, decode_state_payload, encode_state_payload, get_param_value, param_count,
    text_to_value, value_to_text, write_param_info, PumpParams,
};
use crate::time_utils::monotonic_micros;

#[cfg(test)]
mod build_support;
mod curve;
mod dsp;
mod gui;
mod gui_status;
mod params;
mod plugin_processor;
mod time_utils;
mod transport;
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
            .with_vendor("PORTALSURFER")
            .with_features([AUDIO_EFFECT, STEREO])
            .with_description("Node-based beat-synced gain ducking effect")
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

        let _stats = self
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

pub use gui_status::{GuiStatus, GuiTransportTelemetry};
use plugin_processor::PumpAudioProcessor;
use transport::{gui_phase_from_transport, host_beat_phase};

toybox::clap_plugin_entry!(PumpPlugin);
