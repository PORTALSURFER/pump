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
use toybox::dsp::{phase_from_beats, AtomicF32, TransportState};

use crate::dsp::{DspSettings, PumpEngine};
use crate::gui::PumpGui;
use crate::params::{
    apply_param_event, decode_state_payload, encode_state_payload, get_param_value, param_count,
    text_to_value, value_to_text, write_param_info, PumpParams,
};
use crate::time_utils::monotonic_micros;

mod curve;
mod dsp;
mod gui;
mod params;
mod time_utils;
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

/// Resolve GUI phase from the latest host transport snapshot.
///
/// Uses host beat timeline when available so the curve marker remains aligned to
/// arranger position even when no audio frames are processed for this callback.
fn gui_phase_from_transport(
    transport: TransportState,
    settings: DspSettings,
    fallback_phase: f32,
) -> f32 {
    transport
        .song_pos_beats
        .map(|beats| phase_from_beats(beats, settings.beats_per_cycle, settings.phase_offset))
        .unwrap_or_else(|| fallback_phase.rem_euclid(1.0))
}

/// Resolve normalized host beat phase from transport snapshot.
fn host_beat_phase(transport: TransportState) -> Option<f32> {
    transport
        .song_pos_beats
        .map(|beats| beats.rem_euclid(1.0) as f32)
}

/// Extrapolate normalized phase from an anchor phase and elapsed time.
fn extrapolate_phase(
    anchor_phase: f32,
    frequency_hz: f32,
    is_playing: bool,
    last_update_micros: u64,
    now_micros: u64,
) -> f32 {
    let anchor = anchor_phase.rem_euclid(1.0);
    if !is_playing || now_micros <= last_update_micros {
        return anchor;
    }
    let elapsed_seconds = (now_micros - last_update_micros) as f32 / 1_000_000.0;
    (anchor + elapsed_seconds * frequency_hz.max(0.0)).rem_euclid(1.0)
}

/// Transport telemetry payload mirrored from the audio thread to the GUI.
#[derive(Debug, Copy, Clone)]
pub struct GuiTransportTelemetry {
    /// Whether host playback is currently running.
    pub is_playing: bool,
    /// Whether host beat timeline is currently available.
    pub has_host_beats_timeline: bool,
    /// Normalized host beat phase in `[0, 1)`.
    pub beat_phase: f32,
    /// Host tempo in beats per minute.
    pub tempo_bpm: f32,
    /// Pump cycle length in quarter-note beats.
    pub beats_per_cycle: f32,
}

/// Shared GUI telemetry values updated by the audio thread.
#[derive(Default)]
pub struct GuiStatus {
    phase: AtomicF32,
    gain: AtomicF32,
    is_playing: AtomicBool,
    has_host_beats_timeline: AtomicBool,
    beat_phase: AtomicF32,
    tempo_bpm: AtomicF32,
    cycle_hz: AtomicF32,
    last_update_micros: AtomicU64,
}

impl GuiStatus {
    /// Update telemetry from the latest processed frame.
    pub fn update(&self, phase: f32, gain: f32, transport: GuiTransportTelemetry) {
        let safe_tempo = transport.tempo_bpm.clamp(20.0, 320.0);
        let safe_beats_per_cycle = transport.beats_per_cycle.max(1.0e-4);
        self.phase.store(phase, Ordering::Relaxed);
        self.gain.store(gain, Ordering::Relaxed);
        self.is_playing
            .store(transport.is_playing, Ordering::Relaxed);
        self.has_host_beats_timeline
            .store(transport.has_host_beats_timeline, Ordering::Relaxed);
        self.beat_phase
            .store(transport.beat_phase.rem_euclid(1.0), Ordering::Relaxed);
        self.tempo_bpm.store(safe_tempo, Ordering::Relaxed);
        self.cycle_hz.store(
            (safe_tempo / 60.0) / safe_beats_per_cycle,
            Ordering::Relaxed,
        );
        self.last_update_micros
            .store(monotonic_micros(), Ordering::Relaxed);
    }

    /// Read latest phase value.
    pub fn phase(&self) -> f32 {
        extrapolate_phase(
            self.phase.load(Ordering::Relaxed),
            self.cycle_hz.load(Ordering::Relaxed),
            self.is_playing(),
            self.last_update_micros.load(Ordering::Relaxed),
            monotonic_micros(),
        )
    }

    /// Read latest linear gain value.
    pub fn gain(&self) -> f32 {
        self.gain.load(Ordering::Relaxed).max(0.0)
    }

    /// Read whether host transport is currently playing.
    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    /// Read whether host beat timeline is currently available.
    pub fn has_host_beats_timeline(&self) -> bool {
        self.has_host_beats_timeline.load(Ordering::Relaxed)
    }

    /// Read normalized host beat phase in `[0, 1)`.
    pub fn beat_phase(&self) -> f32 {
        extrapolate_phase(
            self.beat_phase.load(Ordering::Relaxed),
            self.tempo_bpm.load(Ordering::Relaxed) / 60.0,
            self.is_playing(),
            self.last_update_micros.load(Ordering::Relaxed),
            monotonic_micros(),
        )
    }

    /// Return whether transport beat blink should currently be lit.
    pub fn transport_beat_blink_active(&self) -> bool {
        const BEAT_FLASH_DUTY: f32 = 0.18;
        if !self.is_playing() {
            return false;
        }
        if self.has_host_beats_timeline() {
            return self.beat_phase() < BEAT_FLASH_DUTY;
        }
        // Fallback activity mode: keep the transport indicator lit while
        // playing when the host does not expose a beat timeline.
        true
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
    use crate::dsp::{db_to_linear, DspSettings};
    use std::sync::atomic::Ordering;
    use toybox::dsp::{phase_from_beats, TransportState};

    use super::{
        extrapolate_phase, gui_phase_from_transport, host_beat_phase, monotonic_micros, GuiStatus,
        GuiTransportTelemetry,
    };

    #[test]
    fn db_to_linear_matches_unity_at_zero_db() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn gui_phase_from_transport_prefers_host_song_position() {
        let settings = DspSettings {
            mix: 1.0,
            depth: 1.0,
            phase_offset: 0.2,
            output_gain_db: 0.0,
            beats_per_cycle: 4.0,
        };
        let transport = TransportState {
            tempo_bpm: 128.0,
            is_playing: true,
            song_pos_beats: Some(9.5),
        };
        let expected = phase_from_beats(9.5, settings.beats_per_cycle, settings.phase_offset);
        let resolved = gui_phase_from_transport(transport, settings, 0.75);
        assert!((resolved - expected).abs() < 1.0e-6);
    }

    #[test]
    fn gui_phase_from_transport_uses_fallback_without_song_position() {
        let settings = DspSettings {
            mix: 1.0,
            depth: 1.0,
            phase_offset: 0.0,
            output_gain_db: 0.0,
            beats_per_cycle: 1.0,
        };
        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: false,
            song_pos_beats: None,
        };
        let resolved = gui_phase_from_transport(transport, settings, 1.25);
        assert!((resolved - 0.25).abs() < 1.0e-6);
    }

    #[test]
    fn host_beat_phase_wraps_negative_and_positive_positions() {
        let positive = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(4.75),
        };
        let negative = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: Some(-0.2),
        };
        assert!((host_beat_phase(positive).unwrap_or_default() - 0.75).abs() < 1.0e-6);
        assert!((host_beat_phase(negative).unwrap_or_default() - 0.8).abs() < 1.0e-6);
    }

    #[test]
    fn transport_beat_blink_requires_playback_and_uses_timeline_when_available() {
        let status = GuiStatus::default();
        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.05,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        assert!(status.transport_beat_blink_active());

        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.5,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        assert!(!status.transport_beat_blink_active());

        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: false,
                has_host_beats_timeline: true,
                beat_phase: 0.05,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        assert!(!status.transport_beat_blink_active());

        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: false,
                beat_phase: 0.05,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        assert!(status.transport_beat_blink_active());
    }

    #[test]
    fn extrapolate_phase_advances_when_playing() {
        let phase = extrapolate_phase(0.25, 2.0, true, 1_000_000, 1_250_000);
        assert!((phase - 0.75).abs() < 1.0e-6);
    }

    #[test]
    fn extrapolate_phase_holds_when_not_playing() {
        let phase = extrapolate_phase(0.25, 2.0, false, 1_000_000, 2_000_000);
        assert!((phase - 0.25).abs() < 1.0e-6);
    }

    #[test]
    fn gui_status_phase_holds_when_last_update_timestamp_is_stale_future_value() {
        let status = GuiStatus::default();
        status.update(
            0.42,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.2,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        status.last_update_micros.store(
            monotonic_micros().saturating_add(1_000_000),
            Ordering::Relaxed,
        );
        let phase = status.phase();
        assert!(
            (phase - 0.42).abs() <= 1.0e-6,
            "future/stale timestamp should hold anchor phase instead of extrapolating"
        );
    }

    #[test]
    fn gui_status_beat_phase_holds_when_last_update_timestamp_is_stale_future_value() {
        let status = GuiStatus::default();
        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.73,
                tempo_bpm: 123.0,
                beats_per_cycle: 1.0,
            },
        );
        status.last_update_micros.store(
            monotonic_micros().saturating_add(1_000_000),
            Ordering::Relaxed,
        );
        let phase = status.beat_phase();
        assert!(
            (phase - 0.73).abs() <= 1.0e-6,
            "future/stale timestamp should hold anchor beat phase instead of extrapolating"
        );
    }
}
