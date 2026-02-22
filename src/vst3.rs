//! Minimal VST3 adapter for Pump.
//!
//! This adapter shares parameter ranges/state with the CLAP implementation and
//! processes the same gain-envelope DSP core.

use std::ffi::{c_void, CStr};
use std::ptr;
use std::slice;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use toybox::clap::automation::AutomationQueue;
use toybox::vst3::prelude::Steinberg::*;
use toybox::vst3::prelude::*;

use crate::dsp::{DspSettings, PumpEngine};
use crate::gui::{preferred_window_size, PumpGui};
use crate::params::{
    apply_normalized_param_value, clap_id_from_vst3_param_id, decode_state_payload,
    encode_state_payload, format_plain_value_text, get_param_value, normalized_from_plain_value,
    param_count, parse_plain_value_text, plain_from_normalized_value, vst3_param_info_for_index,
    PumpParams, PARAM_DEPTH_NUM, PARAM_MIX_NUM, PARAM_OUTPUT_GAIN_NUM, PARAM_PHASE_OFFSET_NUM,
    PARAM_SYNC_DIVISION_NUM,
};
use crate::plugin_metadata::PLUGIN_NAME;
use crate::transport::{gui_phase_from_transport, gui_transport_telemetry};
use crate::GuiStatus;
use toybox::dsp::TransportState;

const PROCESSOR_CID: TUID = uid(0xE5A9A79F, 0xC4A94392, 0x97A8A8AA, 0xA9A90B3C);
const CONTROLLER_CID: TUID = uid(0xB2EE267A, 0xE4314D5D, 0x96085F7A, 0x51681074);

const STATE_MAGIC: u32 = u32::from_le_bytes(*b"PUMP");
const STATE_VERSION: u32 = 1;

mod param_bridge;
mod shared_state;
mod transport_utils;

use param_bridge::{apply_normalized_param, from_normalized, read_plain_param, to_normalized};
use shared_state::{
    acquire_shared_for_role, release_shared_for_role, PumpVst3Runtime, PumpVst3Shared, SharedRole,
};
use transport_utils::transport_state_from_vst3_process_context;

#[cfg(test)]
use shared_state::{shared_registry, SharedRegistryEntry};

mod controller;
mod factory;
mod gui_adapter;

use controller::PumpVst3Controller;
use gui_adapter::PumpVst3GuiAdapter;

struct PumpVst3Processor {
    shared: Arc<PumpVst3Shared>,
    runtime: Mutex<PumpVst3Runtime>,
}

impl PumpVst3Processor {
    fn new(shared: Arc<PumpVst3Shared>) -> Self {
        Self {
            runtime: Mutex::new(PumpVst3Runtime::new(shared.params.as_ref())),
            shared,
        }
    }
}

impl Drop for PumpVst3Processor {
    fn drop(&mut self) {
        release_shared_for_role(&self.shared, SharedRole::Processor);
    }
}

impl Class for PumpVst3Processor {
    type Interfaces = (IComponent, IAudioProcessor, IProcessContextRequirements);
}

impl IPluginBaseTrait for PumpVst3Processor {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl IComponentTrait for PumpVst3Processor {
    unsafe fn getControllerClassId(&self, class_id: *mut TUID) -> tresult {
        if class_id.is_null() {
            return kInvalidArgument;
        }
        unsafe { *class_id = CONTROLLER_CID };
        kResultOk
    }

    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }

    unsafe fn getBusCount(&self, media_type: MediaType, dir: BusDirection) -> i32 {
        match media_type as MediaTypes {
            MediaTypes_::kAudio => match dir as BusDirections {
                BusDirections_::kInput | BusDirections_::kOutput => 1,
                _ => 0,
            },
            _ => 0,
        }
    }

    #[allow(clippy::unnecessary_cast)]
    unsafe fn getBusInfo(
        &self,
        media_type: MediaType,
        dir: BusDirection,
        index: i32,
        bus: *mut BusInfo,
    ) -> tresult {
        if bus.is_null() || index != 0 {
            return kInvalidArgument;
        }
        if media_type as MediaTypes != MediaTypes_::kAudio {
            return kInvalidArgument;
        }

        let label = match dir as BusDirections {
            BusDirections_::kInput => "Input",
            BusDirections_::kOutput => "Output",
            _ => return kInvalidArgument,
        };

        let bus = unsafe { &mut *bus };
        bus.mediaType = MediaTypes_::kAudio as MediaType;
        bus.direction = dir;
        bus.channelCount = 2;
        copy_wstring(label, &mut bus.name);
        bus.busType = BusTypes_::kMain as BusType;
        bus.flags = {
            #[cfg(windows)]
            {
                BusInfo_::BusFlags_::kDefaultActive as u32
            }
            #[cfg(not(windows))]
            {
                BusInfo_::BusFlags_::kDefaultActive as u32
            }
        };

        kResultOk
    }

    unsafe fn getRoutingInfo(
        &self,
        _in_info: *mut RoutingInfo,
        _out_info: *mut RoutingInfo,
    ) -> tresult {
        kNotImplemented
    }

    unsafe fn activateBus(
        &self,
        _media_type: MediaType,
        _dir: BusDirection,
        _index: i32,
        _state: TBool,
    ) -> tresult {
        kResultOk
    }

    unsafe fn setActive(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        let payload = unsafe { read_versioned_payload(state, STATE_MAGIC, &[STATE_VERSION]) };
        let Ok(payload) = payload else {
            return kInvalidArgument;
        };

        if decode_state_payload(self.shared.params.as_ref(), &payload.payload).is_err() {
            return kInvalidArgument;
        }

        if let Ok(mut runtime) = self.runtime.lock() {
            runtime
                .engine
                .set_target_curve(self.shared.params.curve_snapshot());
            runtime.last_curve_revision = self.shared.params.curve_revision();
        }

        kResultOk
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        let payload = encode_state_payload(self.shared.params.as_ref());
        match unsafe { write_versioned_payload(state, STATE_MAGIC, STATE_VERSION, &payload) } {
            Ok(()) => kResultOk,
            Err(_) => kResultFalse,
        }
    }
}

impl IAudioProcessorTrait for PumpVst3Processor {
    unsafe fn setBusArrangements(
        &self,
        inputs: *mut SpeakerArrangement,
        num_ins: i32,
        outputs: *mut SpeakerArrangement,
        num_outs: i32,
    ) -> tresult {
        if num_ins != 1 || num_outs != 1 {
            return kResultFalse;
        }
        if inputs.is_null() || outputs.is_null() {
            return kInvalidArgument;
        }

        if unsafe { *inputs } != SpeakerArr::kStereo || unsafe { *outputs } != SpeakerArr::kStereo {
            return kResultFalse;
        }

        kResultTrue
    }

    unsafe fn getBusArrangement(
        &self,
        dir: BusDirection,
        index: i32,
        arr: *mut SpeakerArrangement,
    ) -> tresult {
        if arr.is_null() || index != 0 {
            return kInvalidArgument;
        }

        match dir as BusDirections {
            BusDirections_::kInput | BusDirections_::kOutput => {
                unsafe { *arr = SpeakerArr::kStereo };
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn canProcessSampleSize(&self, symbolic_sample_size: i32) -> tresult {
        match symbolic_sample_size as SymbolicSampleSizes {
            SymbolicSampleSizes_::kSample32 => kResultOk,
            SymbolicSampleSizes_::kSample64 => kNotImplemented,
            _ => kInvalidArgument,
        }
    }

    unsafe fn getLatencySamples(&self) -> u32 {
        0
    }

    unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> tresult {
        if setup.is_null() {
            return kInvalidArgument;
        }

        let setup = unsafe { &*setup };
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.set_sample_rate(setup.sampleRate, self.shared.params.as_ref());
        }

        kResultOk
    }

    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        if data.is_null() {
            return kInvalidArgument;
        }

        let process_data = unsafe { &*data };

        for id in [
            PARAM_MIX_NUM,
            PARAM_DEPTH_NUM,
            PARAM_PHASE_OFFSET_NUM,
            PARAM_OUTPUT_GAIN_NUM,
            PARAM_SYNC_DIVISION_NUM,
        ] {
            if let Some((_, value)) =
                unsafe { latest_param_point(process_data.inputParameterChanges, id) }
            {
                apply_normalized_param(self.shared.params.as_ref(), id, value);
            }
        }

        let mut runtime = match self.runtime.lock() {
            Ok(runtime) => runtime,
            Err(_) => return process_ok(),
        };

        let revision = self.shared.params.curve_revision();
        if revision != runtime.last_curve_revision {
            runtime
                .engine
                .set_target_curve(self.shared.params.curve_snapshot());
            runtime.last_curve_revision = revision;
        }

        let settings = DspSettings {
            mix: self.shared.params.mix(),
            depth: self.shared.params.depth(),
            phase_offset: self.shared.params.phase_offset(),
            output_gain_db: self.shared.params.output_gain_db(),
            beats_per_cycle: self.shared.params.sync_beats_per_cycle(),
        };
        let transport = transport_state_from_vst3_process_context(process_data.processContext);
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

        let Some(buffers) = (unsafe { stereo_f32_buffers(process_data) }) else {
            return process_ok();
        };

        let mut last_phase = 0.0_f32;
        let mut last_gain = 1.0_f32;
        let mut transport_for_sample = transport;
        for sample_index in 0..buffers.num_samples {
            let mut left = buffers.input_left[sample_index];
            let mut right = buffers.input_right[sample_index];
            let telemetry = runtime.engine.process_sample(
                &mut left,
                &mut right,
                settings,
                transport_for_sample,
            );
            transport_for_sample.song_pos_beats = None;
            last_phase = telemetry.phase;
            last_gain = telemetry.gain;
            buffers.output_left[sample_index] = left;
            buffers.output_right[sample_index] = right;
        }
        self.shared.status.update(
            last_phase,
            last_gain,
            gui_transport_telemetry(
                transport,
                settings.beats_per_cycle,
                self.shared.status.beat_phase(),
            ),
        );

        process_ok()
    }

    unsafe fn getTailSamples(&self) -> u32 {
        0
    }
}

impl IProcessContextRequirementsTrait for PumpVst3Processor {
    unsafe fn getProcessContextRequirements(&self) -> u32 {
        IProcessContextRequirements_::Flags_::kNeedTempo
            | IProcessContextRequirements_::Flags_::kNeedProjectTimeMusic
            | IProcessContextRequirements_::Flags_::kNeedTransportState
    }
}

#[cfg(test)]
mod tests;
