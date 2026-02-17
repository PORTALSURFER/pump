//! Minimal VST3 adapter for Pump.
//!
//! This adapter shares parameter ranges/state with the CLAP implementation and
//! processes the same gain-envelope DSP core.

#![allow(clippy::missing_docs_in_private_items)]

use std::ffi::{c_void, CStr};
use std::slice;
use std::ptr;
use std::sync::{Arc, Mutex};

use toybox::clap::automation::AutomationQueue;
use toybox::vst3::prelude::Steinberg::*;
use toybox::vst3::prelude::*;

use crate::dsp::{DspSettings, PumpEngine};
use crate::gui::{preferred_window_size, PumpGui};
use crate::params::{
    decode_state_payload, encode_state_payload, sync_division_index_from_text, sync_division_label,
    PumpParams, DEFAULT_DEPTH, DEFAULT_MIX, DEFAULT_OUTPUT_GAIN_DB, DEFAULT_PHASE_OFFSET,
    DEFAULT_SYNC_DIVISION_INDEX, MAX_DEPTH, MAX_MIX, MAX_OUTPUT_GAIN_DB, MAX_PHASE_OFFSET,
    MAX_SYNC_DIVISION, MIN_DEPTH, MIN_MIX, MIN_OUTPUT_GAIN_DB, MIN_PHASE_OFFSET,
};
use toybox::dsp::TransportState;
use crate::GuiStatus;

const PLUGIN_NAME: &str = "pump";
const PROCESSOR_CID: TUID = uid(0xE5A9A79F, 0xC4A94392, 0x97A8A8AA, 0xA9A90B3C);
const CONTROLLER_CID: TUID = uid(0xB2EE267A, 0xE4314D5D, 0x96085F7A, 0x51681074);

const STATE_MAGIC: u32 = u32::from_le_bytes(*b"PUMP");
const STATE_VERSION: u32 = 1;

const PARAM_MIX_ID: ParamID = 1;
const PARAM_DEPTH_ID: ParamID = 2;
const PARAM_PHASE_OFFSET_ID: ParamID = 3;
const PARAM_OUTPUT_GAIN_ID: ParamID = 4;
const PARAM_SYNC_DIVISION_ID: ParamID = 5;

fn to_normalized(param_id: ParamID, plain: f64) -> f64 {
    match param_id {
        PARAM_MIX_ID => ParamRange::new(MIN_MIX as f64, MAX_MIX as f64).plain_to_normalized(plain),
        PARAM_DEPTH_ID => {
            ParamRange::new(MIN_DEPTH as f64, MAX_DEPTH as f64).plain_to_normalized(plain)
        }
        PARAM_PHASE_OFFSET_ID => ParamRange::new(MIN_PHASE_OFFSET as f64, MAX_PHASE_OFFSET as f64)
            .plain_to_normalized(plain),
        PARAM_OUTPUT_GAIN_ID => {
            ParamRange::new(MIN_OUTPUT_GAIN_DB as f64, MAX_OUTPUT_GAIN_DB as f64)
                .plain_to_normalized(plain)
        }
        PARAM_SYNC_DIVISION_ID => {
            ParamRange::new(0.0, MAX_SYNC_DIVISION as f64).plain_to_normalized(plain)
        }
        _ => 0.0,
    }
}

fn from_normalized(param_id: ParamID, normalized: f64) -> f64 {
    match param_id {
        PARAM_MIX_ID => {
            ParamRange::new(MIN_MIX as f64, MAX_MIX as f64).normalized_to_plain(normalized)
        }
        PARAM_DEPTH_ID => {
            ParamRange::new(MIN_DEPTH as f64, MAX_DEPTH as f64).normalized_to_plain(normalized)
        }
        PARAM_PHASE_OFFSET_ID => ParamRange::new(MIN_PHASE_OFFSET as f64, MAX_PHASE_OFFSET as f64)
            .normalized_to_plain(normalized),
        PARAM_OUTPUT_GAIN_ID => {
            ParamRange::new(MIN_OUTPUT_GAIN_DB as f64, MAX_OUTPUT_GAIN_DB as f64)
                .normalized_to_plain(normalized)
        }
        PARAM_SYNC_DIVISION_ID => ParamRange::new(0.0, MAX_SYNC_DIVISION as f64)
            .normalized_to_plain(normalized)
            .round(),
        _ => 0.0,
    }
}

fn read_plain_param(params: &PumpParams, param_id: ParamID) -> f64 {
    match param_id {
        PARAM_MIX_ID => params.mix() as f64,
        PARAM_DEPTH_ID => params.depth() as f64,
        PARAM_PHASE_OFFSET_ID => params.phase_offset() as f64,
        PARAM_OUTPUT_GAIN_ID => params.output_gain_db() as f64,
        PARAM_SYNC_DIVISION_ID => params.sync_division() as f64,
        _ => 0.0,
    }
}

fn apply_plain_param(params: &PumpParams, param_id: ParamID, plain: f64) {
    match param_id {
        PARAM_MIX_ID => params.set_mix(plain as f32),
        PARAM_DEPTH_ID => params.set_depth(plain as f32),
        PARAM_PHASE_OFFSET_ID => params.set_phase_offset(plain as f32),
        PARAM_OUTPUT_GAIN_ID => params.set_output_gain_db(plain as f32),
        PARAM_SYNC_DIVISION_ID => params.set_sync_division(plain as f32),
        _ => {}
    }
}

fn apply_normalized_param(params: &PumpParams, param_id: ParamID, normalized: f64) {
    let plain = from_normalized(param_id, normalized);
    apply_plain_param(params, param_id, plain);
}

struct PumpVst3Runtime {
    engine: PumpEngine,
    last_curve_revision: u32,
    sample_rate: f32,
}

impl PumpVst3Runtime {
    fn new(params: &PumpParams) -> Self {
        let curve = params.curve_snapshot();
        Self {
            engine: PumpEngine::new(48_000.0, curve),
            last_curve_revision: params.curve_revision(),
            sample_rate: 48_000.0,
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f64, params: &PumpParams) {
        let clamped = sample_rate.max(1.0) as f32;
        if (self.sample_rate - clamped).abs() < 1.0e-6 {
            return;
        }

        self.sample_rate = clamped;
        self.engine = PumpEngine::new(self.sample_rate, params.curve_snapshot());
        self.last_curve_revision = params.curve_revision();
    }
}

struct PumpVst3Processor {
    params: Arc<PumpParams>,
    runtime: Mutex<PumpVst3Runtime>,
}

impl PumpVst3Processor {
    fn new() -> Self {
        let params = Arc::new(PumpParams::new());
        Self {
            runtime: Mutex::new(PumpVst3Runtime::new(params.as_ref())),
            params,
        }
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
                BusInfo_::BusFlags_::kDefaultActive
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

        if decode_state_payload(self.params.as_ref(), &payload.payload).is_err() {
            return kInvalidArgument;
        }

        if let Ok(mut runtime) = self.runtime.lock() {
            runtime
                .engine
                .set_target_curve(self.params.curve_snapshot());
            runtime.last_curve_revision = self.params.curve_revision();
        }

        kResultOk
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        let payload = encode_state_payload(self.params.as_ref());
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
            runtime.set_sample_rate(setup.sampleRate, self.params.as_ref());
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
            PARAM_MIX_ID,
            PARAM_DEPTH_ID,
            PARAM_PHASE_OFFSET_ID,
            PARAM_OUTPUT_GAIN_ID,
            PARAM_SYNC_DIVISION_ID,
        ] {
            if let Some((_, value)) =
                unsafe { latest_param_point(process_data.inputParameterChanges, id) }
            {
                apply_normalized_param(self.params.as_ref(), id, value);
            }
        }

        let Some(buffers) = (unsafe { stereo_f32_buffers(process_data) }) else {
            return process_ok();
        };

        let mut runtime = match self.runtime.lock() {
            Ok(runtime) => runtime,
            Err(_) => return process_ok(),
        };

        let revision = self.params.curve_revision();
        if revision != runtime.last_curve_revision {
            runtime
                .engine
                .set_target_curve(self.params.curve_snapshot());
            runtime.last_curve_revision = revision;
        }

        let settings = DspSettings {
            mix: self.params.mix(),
            depth: self.params.depth(),
            phase_offset: self.params.phase_offset(),
            output_gain_db: self.params.output_gain_db(),
            beats_per_cycle: self.params.sync_beats_per_cycle(),
        };

        let transport = TransportState {
            tempo_bpm: 120.0,
            is_playing: true,
            song_pos_beats: None,
        };

        for sample_index in 0..buffers.num_samples {
            let mut left = buffers.input_left[sample_index];
            let mut right = buffers.input_right[sample_index];
            let _ = runtime
                .engine
                .process_sample(&mut left, &mut right, settings, transport);
            buffers.output_left[sample_index] = left;
            buffers.output_right[sample_index] = right;
        }

        process_ok()
    }

    unsafe fn getTailSamples(&self) -> u32 {
        0
    }
}

impl IProcessContextRequirementsTrait for PumpVst3Processor {
    unsafe fn getProcessContextRequirements(&self) -> u32 {
        0
    }
}

struct PumpVst3Controller {
    params: Arc<PumpParams>,
}

impl PumpVst3Controller {
    fn new() -> Self {
        Self {
            params: Arc::new(PumpParams::new()),
        }
    }
}

impl Class for PumpVst3Controller {
    type Interfaces = (IEditController,);
}

impl IPluginBaseTrait for PumpVst3Controller {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl IEditControllerTrait for PumpVst3Controller {
    unsafe fn setComponentState(&self, state: *mut IBStream) -> tresult {
        let payload = unsafe { read_versioned_payload(state, STATE_MAGIC, &[STATE_VERSION]) };
        let Ok(payload) = payload else {
            return kInvalidArgument;
        };

        if decode_state_payload(self.params.as_ref(), &payload.payload).is_ok() {
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        unsafe { self.setComponentState(state) }
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        let payload = encode_state_payload(self.params.as_ref());
        match unsafe { write_versioned_payload(state, STATE_MAGIC, STATE_VERSION, &payload) } {
            Ok(()) => kResultOk,
            Err(_) => kResultFalse,
        }
    }

    unsafe fn getParameterCount(&self) -> int32 {
        5
    }

    unsafe fn getParameterInfo(&self, param_index: int32, info: *mut ParameterInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let info = unsafe { &mut *info };
        match param_index {
            0 => {
                info.id = PARAM_MIX_ID;
                copy_wstring("Mix", &mut info.title);
                copy_wstring("Mix", &mut info.shortTitle);
                copy_wstring("%", &mut info.units);
                info.stepCount = 0;
                info.defaultNormalizedValue = to_normalized(PARAM_MIX_ID, DEFAULT_MIX as f64);
                info.unitId = 0;
                info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
                kResultOk
            }
            1 => {
                info.id = PARAM_DEPTH_ID;
                copy_wstring("Depth", &mut info.title);
                copy_wstring("Depth", &mut info.shortTitle);
                copy_wstring("%", &mut info.units);
                info.stepCount = 0;
                info.defaultNormalizedValue = to_normalized(PARAM_DEPTH_ID, DEFAULT_DEPTH as f64);
                info.unitId = 0;
                info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
                kResultOk
            }
            2 => {
                info.id = PARAM_PHASE_OFFSET_ID;
                copy_wstring("Phase Offset", &mut info.title);
                copy_wstring("Phase", &mut info.shortTitle);
                copy_wstring("%", &mut info.units);
                info.stepCount = 0;
                info.defaultNormalizedValue =
                    to_normalized(PARAM_PHASE_OFFSET_ID, DEFAULT_PHASE_OFFSET as f64);
                info.unitId = 0;
                info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
                kResultOk
            }
            3 => {
                info.id = PARAM_OUTPUT_GAIN_ID;
                copy_wstring("Output", &mut info.title);
                copy_wstring("Output", &mut info.shortTitle);
                copy_wstring("dB", &mut info.units);
                info.stepCount = 0;
                info.defaultNormalizedValue =
                    to_normalized(PARAM_OUTPUT_GAIN_ID, DEFAULT_OUTPUT_GAIN_DB as f64);
                info.unitId = 0;
                info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
                kResultOk
            }
            4 => {
                info.id = PARAM_SYNC_DIVISION_ID;
                copy_wstring("Division", &mut info.title);
                copy_wstring("Division", &mut info.shortTitle);
                copy_wstring("", &mut info.units);
                info.stepCount = MAX_SYNC_DIVISION as i32;
                info.defaultNormalizedValue =
                    to_normalized(PARAM_SYNC_DIVISION_ID, DEFAULT_SYNC_DIVISION_INDEX as f64);
                info.unitId = 0;
                info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn getParamStringByValue(
        &self,
        id: ParamID,
        value_normalized: ParamValue,
        string: *mut String128,
    ) -> tresult {
        if string.is_null() {
            return kInvalidArgument;
        }

        let plain = from_normalized(id, value_normalized);
        let display = match id {
            PARAM_MIX_ID | PARAM_DEPTH_ID | PARAM_PHASE_OFFSET_ID => {
                format!("{:.0}%", plain * 100.0)
            }
            PARAM_OUTPUT_GAIN_ID => format!("{plain:+.1} dB"),
            PARAM_SYNC_DIVISION_ID => sync_division_label(plain as usize).to_string(),
            _ => String::new(),
        };
        copy_wstring(&display, unsafe { &mut *string });
        kResultOk
    }

    unsafe fn getParamValueByString(
        &self,
        id: ParamID,
        string: *mut TChar,
        value_normalized: *mut ParamValue,
    ) -> tresult {
        if value_normalized.is_null() {
            return kInvalidArgument;
        }

        let value = match id {
            PARAM_SYNC_DIVISION_ID => {
                if string.is_null() {
                    return kInvalidArgument;
                }
                let len = unsafe { tchar_len(string) };
                let utf16 = unsafe { slice::from_raw_parts(string.cast::<u16>(), len) };
                let Some(parsed) = String::from_utf16(utf16).ok() else {
                    return kInvalidArgument;
                };
                let Some(index) = sync_division_index_from_text(parsed.trim()) else {
                    return kInvalidArgument;
                };
                to_normalized(id, index as f64)
            }
            PARAM_MIX_ID | PARAM_DEPTH_ID | PARAM_PHASE_OFFSET_ID => {
                let Some(parsed) = (unsafe { parse_tchar_f64(string) }) else {
                    return kInvalidArgument;
                };
                to_normalized(id, (parsed / 100.0).clamp(0.0, 1.0))
            }
            _ => {
                let Some(parsed) = (unsafe { parse_tchar_f64(string) }) else {
                    return kInvalidArgument;
                };
                to_normalized(id, parsed)
            }
        };

        unsafe { *value_normalized = value };
        kResultOk
    }

    unsafe fn normalizedParamToPlain(
        &self,
        id: ParamID,
        value_normalized: ParamValue,
    ) -> ParamValue {
        from_normalized(id, value_normalized)
    }

    unsafe fn plainParamToNormalized(&self, id: ParamID, plain_value: ParamValue) -> ParamValue {
        to_normalized(id, plain_value)
    }

    unsafe fn getParamNormalized(&self, id: ParamID) -> ParamValue {
        to_normalized(id, read_plain_param(self.params.as_ref(), id))
    }

    unsafe fn setParamNormalized(&self, id: ParamID, value: ParamValue) -> tresult {
        apply_normalized_param(self.params.as_ref(), id, value);
        kResultOk
    }

    unsafe fn setComponentHandler(&self, _handler: *mut IComponentHandler) -> tresult {
        kResultOk
    }

    unsafe fn createView(&self, name: FIDString) -> *mut IPlugView {
        if name.is_null() {
            return ptr::null_mut();
        }

        let requested = unsafe { CStr::from_ptr(name) };
        let editor = unsafe { CStr::from_ptr(ViewType::kEditor) };
        if requested.to_bytes() != editor.to_bytes() {
            return ptr::null_mut();
        }

        let adapter = PumpVst3GuiAdapter::new(self.params.clone());
        let (default_width, default_height) = preferred_window_size();
        let Some(view) =
            ComWrapper::new(
                HostedVst3View::new(adapter, default_width, default_height),
            )
                .to_com_ptr::<IPlugView>()
        else {
            return ptr::null_mut();
        };
        ComPtr::into_raw(view)
    }
}

struct PumpVst3GuiAdapter {
    params: Arc<PumpParams>,
    status: Arc<GuiStatus>,
    automation_queue: Arc<AutomationQueue>,
    gui: PumpGui,
}

impl PumpVst3GuiAdapter {
    fn new(params: Arc<PumpParams>) -> Self {
        Self {
            params,
            status: Arc::new(GuiStatus::default()),
            automation_queue: Arc::new(AutomationQueue::default()),
            gui: PumpGui::default(),
        }
    }
}

impl Vst3HostedGui for PumpVst3GuiAdapter {
    fn set_parent_raw(&mut self, parent: toybox::raw_window_handle::RawWindowHandle) {
        self.gui.set_parent_raw(parent);
    }

    fn open(&mut self) -> bool {
        self.gui
            .open(
                &self.params,
                &self.status,
                self.automation_queue.clone(),
                None,
            )
            .is_ok()
    }

    fn close(&mut self) {
        self.gui.close();
    }

    fn last_size(&self) -> Option<(u32, u32)> {
        self.gui.last_size()
    }

    fn request_resize(&self, width: u32, height: u32) {
        self.gui.request_resize(width, height);
    }
}

#[derive(Default)]
struct PumpVst3Factory;

impl Class for PumpVst3Factory {
    type Interfaces = (IPluginFactory,);
}

impl IPluginFactoryTrait for PumpVst3Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let info = unsafe { &mut *info };
        copy_cstring("uhx", &mut info.vendor);
        copy_cstring("https://github.com/uhx/pump", &mut info.url);
        copy_cstring("support@localhost", &mut info.email);
        info.flags = PFactoryInfo_::FactoryFlags_::kUnicode as int32;

        kResultOk
    }

    unsafe fn countClasses(&self) -> i32 {
        2
    }

    unsafe fn getClassInfo(&self, index: i32, info: *mut PClassInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let info = unsafe { &mut *info };
        match index {
            0 => {
                write_class_info_many(
                    info,
                    PROCESSOR_CID,
                    CATEGORY_AUDIO_MODULE_CLASS,
                    PLUGIN_NAME,
                );
                kResultOk
            }
            1 => {
                write_class_info_many(
                    info,
                    CONTROLLER_CID,
                    CATEGORY_COMPONENT_CONTROLLER_CLASS,
                    PLUGIN_NAME,
                );
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn createInstance(
        &self,
        cid: FIDString,
        iid: FIDString,
        obj: *mut *mut c_void,
    ) -> tresult {
        if cid.is_null() || iid.is_null() || obj.is_null() {
            return kInvalidArgument;
        }

        let class_id = unsafe { *(cid as *const TUID) };
        let instance = match class_id {
            PROCESSOR_CID => ComWrapper::new(PumpVst3Processor::new()).to_com_ptr::<FUnknown>(),
            CONTROLLER_CID => ComWrapper::new(PumpVst3Controller::new()).to_com_ptr::<FUnknown>(),
            _ => None,
        };

        let Some(instance) = instance else {
            return kInvalidArgument;
        };

        let ptr = instance.as_ptr();
        unsafe { ((*(*ptr).vtbl).queryInterface)(ptr, iid as *mut TUID, obj) }
    }
}

toybox::vst3_plugin_entry!(PumpVst3Factory);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_reports_expected_parameter_count() {
        let controller = PumpVst3Controller::new();
        let count = unsafe { controller.getParameterCount() };
        assert_eq!(count, 5);
    }

    #[test]
    fn view_enforces_minimum_size() {
        let (preferred_width, preferred_height) = preferred_window_size();
        let view = HostedVst3View::new(
            PumpVst3GuiAdapter::new(Arc::new(PumpParams::new())),
            preferred_width,
            preferred_height,
        );
        let mut rect = view_rect(10, 10);
        let result = unsafe { view.checkSizeConstraint(&mut rect) };
        assert_eq!(result, kResultOk);
        assert!(rect.right - rect.left >= preferred_width as i32);
        assert!(rect.bottom - rect.top >= preferred_height as i32);
    }
}
