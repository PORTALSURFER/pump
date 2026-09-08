use super::*;
use crate::gui::HostParamEditSink;
use crate::params::{
    PumpParams, DEFAULT_FREE_RATE_HZ, PARAM_FREE_RATE_NUM, PARAM_PHASE_OFFSET_NUM, PARAM_SWING_NUM,
    PARAM_SYNC_DIVISION_ID, PARAM_SYNC_DIVISION_NUM, PARAM_SYNC_DIVISION_VST3_V2_NUM,
    PARAM_TIMING_MODE_NUM, TIMING_MODE_FREE, TIMING_MODE_SYNC,
};
use std::ffi::c_void;
use std::mem;
use std::ptr;
use std::sync::Mutex as StdMutex;

#[repr(C)]
struct TestVst3Stream {
    base: IBStream,
    data: Vec<u8>,
    cursor: usize,
}

impl TestVst3Stream {
    fn v14(payload: Vec<u8>) -> Self {
        let mut data = Vec::with_capacity(12 + payload.len());
        data.extend_from_slice(&STATE_MAGIC.to_le_bytes());
        data.extend_from_slice(&14u32.to_le_bytes());
        data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        data.extend_from_slice(&payload);
        Self {
            base: IBStream {
                vtbl: &TEST_STREAM_VTABLE,
            },
            data,
            cursor: 0,
        }
    }

    fn as_ptr(&mut self) -> *mut IBStream {
        &mut self.base
    }
}

static TEST_STREAM_VTABLE: IBStreamVtbl = IBStreamVtbl {
    base: FUnknownVtbl {
        queryInterface: test_stream_query_interface,
        addRef: test_stream_add_ref,
        release: test_stream_release,
    },
    read: test_stream_read,
    write: test_stream_write,
    seek: test_stream_seek,
    tell: test_stream_tell,
};

unsafe extern "system" fn test_stream_query_interface(
    _this: *mut FUnknown,
    _iid: *const TUID,
    obj: *mut *mut c_void,
) -> tresult {
    if !obj.is_null() {
        unsafe { *obj = ptr::null_mut() };
    }
    kNoInterface
}

unsafe extern "system" fn test_stream_add_ref(_this: *mut FUnknown) -> u32 {
    1
}

unsafe extern "system" fn test_stream_release(_this: *mut FUnknown) -> u32 {
    1
}

unsafe extern "system" fn test_stream_read(
    this: *mut IBStream,
    buffer: *mut c_void,
    num_bytes: int32,
    num_bytes_read: *mut int32,
) -> tresult {
    let stream = unsafe { &mut *(this as *mut TestVst3Stream) };
    let available = stream.data.len().saturating_sub(stream.cursor);
    let count = (num_bytes.max(0) as usize).min(available);
    if count > 0 {
        unsafe {
            ptr::copy_nonoverlapping(
                stream.data.as_ptr().add(stream.cursor),
                buffer.cast::<u8>(),
                count,
            );
        }
        stream.cursor += count;
    }
    if !num_bytes_read.is_null() {
        unsafe { *num_bytes_read = count as int32 };
    }
    kResultOk
}

unsafe extern "system" fn test_stream_write(
    _this: *mut IBStream,
    _buffer: *mut c_void,
    _num_bytes: int32,
    num_bytes_written: *mut int32,
) -> tresult {
    if !num_bytes_written.is_null() {
        unsafe { *num_bytes_written = 0 };
    }
    kResultFalse
}

unsafe extern "system" fn test_stream_seek(
    this: *mut IBStream,
    pos: int64,
    _mode: int32,
    result: *mut int64,
) -> tresult {
    if pos < 0 {
        return kInvalidArgument;
    }
    let stream = unsafe { &mut *(this as *mut TestVst3Stream) };
    stream.cursor = pos as usize;
    if !result.is_null() {
        unsafe { *result = stream.cursor as int64 };
    }
    kResultOk
}

unsafe extern "system" fn test_stream_tell(this: *mut IBStream, result: *mut int64) -> tresult {
    let stream = unsafe { &mut *(this as *mut TestVst3Stream) };
    if !result.is_null() {
        unsafe { *result = stream.cursor as int64 };
    }
    kResultOk
}

#[derive(Clone, Debug, PartialEq)]
enum RecordedEditCall {
    Begin(ParamID),
    Perform(ParamID, ParamValue),
    End(ParamID),
}

struct RecordingComponentHandler {
    calls: Arc<StdMutex<Vec<RecordedEditCall>>>,
    begin_result: tresult,
    perform_result: tresult,
    end_result: tresult,
}

impl Class for RecordingComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for RecordingComponentHandler {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        self.calls
            .lock()
            .expect("recording handler lock")
            .push(RecordedEditCall::Begin(id));
        self.begin_result
    }

    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        self.calls
            .lock()
            .expect("recording handler lock")
            .push(RecordedEditCall::Perform(id, value));
        self.perform_result
    }

    unsafe fn endEdit(&self, id: ParamID) -> tresult {
        self.calls
            .lock()
            .expect("recording handler lock")
            .push(RecordedEditCall::End(id));
        self.end_result
    }

    unsafe fn restartComponent(&self, _flags: i32) -> tresult {
        kResultOk
    }
}

#[repr(C)]
struct TestParamValueQueue {
    base: IParamValueQueue,
    param_id: ParamID,
    points: Vec<(int32, ParamValue)>,
}

impl TestParamValueQueue {
    fn new(param_id: ParamID, points: Vec<(int32, ParamValue)>) -> Self {
        Self {
            base: IParamValueQueue {
                vtbl: &TEST_PARAM_VALUE_QUEUE_VTABLE,
            },
            param_id,
            points,
        }
    }
}

static TEST_PARAM_VALUE_QUEUE_VTABLE: IParamValueQueueVtbl = IParamValueQueueVtbl {
    base: FUnknownVtbl {
        queryInterface: test_param_queue_query_interface,
        addRef: test_param_queue_add_ref,
        release: test_param_queue_release,
    },
    getParameterId: test_param_queue_get_parameter_id,
    getPointCount: test_param_queue_get_point_count,
    getPoint: test_param_queue_get_point,
    addPoint: test_param_queue_add_point,
};

unsafe extern "system" fn test_param_queue_query_interface(
    _this: *mut FUnknown,
    _iid: *const TUID,
    obj: *mut *mut c_void,
) -> tresult {
    if !obj.is_null() {
        unsafe { *obj = ptr::null_mut() };
    }
    kNoInterface
}

unsafe extern "system" fn test_param_queue_add_ref(_this: *mut FUnknown) -> u32 {
    1
}

unsafe extern "system" fn test_param_queue_release(_this: *mut FUnknown) -> u32 {
    1
}

unsafe extern "system" fn test_param_queue_get_parameter_id(
    this: *mut IParamValueQueue,
) -> ParamID {
    unsafe { (&*(this as *const TestParamValueQueue)).param_id }
}

unsafe extern "system" fn test_param_queue_get_point_count(this: *mut IParamValueQueue) -> int32 {
    let queue = unsafe { &*(this as *const TestParamValueQueue) };
    int32::try_from(queue.points.len()).unwrap_or(int32::MAX)
}

unsafe extern "system" fn test_param_queue_get_point(
    this: *mut IParamValueQueue,
    index: int32,
    sample_offset: *mut int32,
    value: *mut ParamValue,
) -> tresult {
    let Some(index) = usize::try_from(index).ok() else {
        return kInvalidArgument;
    };
    let queue = unsafe { &*(this as *const TestParamValueQueue) };
    let Some((point_offset, point_value)) = queue.points.get(index).copied() else {
        return kInvalidArgument;
    };
    if sample_offset.is_null() || value.is_null() {
        return kInvalidArgument;
    }
    unsafe {
        *sample_offset = point_offset;
        *value = point_value;
    }
    kResultTrue
}

unsafe extern "system" fn test_param_queue_add_point(
    _this: *mut IParamValueQueue,
    _sample_offset: int32,
    _value: ParamValue,
    _index: *mut int32,
) -> tresult {
    kNotImplemented
}

#[repr(C)]
struct TestParameterChanges {
    base: IParameterChanges,
    queues: Vec<TestParamValueQueue>,
}

impl TestParameterChanges {
    fn new(queues: Vec<(ParamID, Vec<(int32, ParamValue)>)>) -> Self {
        Self {
            base: IParameterChanges {
                vtbl: &TEST_PARAMETER_CHANGES_VTABLE,
            },
            queues: queues
                .into_iter()
                .map(|(param_id, points)| TestParamValueQueue::new(param_id, points))
                .collect(),
        }
    }

    fn as_ptr(&mut self) -> *mut IParameterChanges {
        &mut self.base
    }
}

static TEST_PARAMETER_CHANGES_VTABLE: IParameterChangesVtbl = IParameterChangesVtbl {
    base: FUnknownVtbl {
        queryInterface: test_parameter_changes_query_interface,
        addRef: test_parameter_changes_add_ref,
        release: test_parameter_changes_release,
    },
    getParameterCount: test_parameter_changes_get_parameter_count,
    getParameterData: test_parameter_changes_get_parameter_data,
    addParameterData: test_parameter_changes_add_parameter_data,
};

unsafe extern "system" fn test_parameter_changes_query_interface(
    _this: *mut FUnknown,
    _iid: *const TUID,
    obj: *mut *mut c_void,
) -> tresult {
    if !obj.is_null() {
        unsafe { *obj = ptr::null_mut() };
    }
    kNoInterface
}

unsafe extern "system" fn test_parameter_changes_add_ref(_this: *mut FUnknown) -> u32 {
    1
}

unsafe extern "system" fn test_parameter_changes_release(_this: *mut FUnknown) -> u32 {
    1
}

unsafe extern "system" fn test_parameter_changes_get_parameter_count(
    this: *mut IParameterChanges,
) -> int32 {
    let changes = unsafe { &*(this as *const TestParameterChanges) };
    int32::try_from(changes.queues.len()).unwrap_or(int32::MAX)
}

unsafe extern "system" fn test_parameter_changes_get_parameter_data(
    this: *mut IParameterChanges,
    index: int32,
) -> *mut IParamValueQueue {
    let Some(index) = usize::try_from(index).ok() else {
        return ptr::null_mut();
    };
    let changes = unsafe { &mut *(this as *mut TestParameterChanges) };
    changes
        .queues
        .get_mut(index)
        .map(|queue| &mut queue.base as *mut IParamValueQueue)
        .unwrap_or(ptr::null_mut())
}

unsafe extern "system" fn test_parameter_changes_add_parameter_data(
    _this: *mut IParameterChanges,
    _id: *const ParamID,
    _index: *mut int32,
) -> *mut IParamValueQueue {
    ptr::null_mut()
}

struct StereoProcessFixture {
    process_data: ProcessData,
    _input_left: Vec<f32>,
    _input_right: Vec<f32>,
    output_left: Vec<f32>,
    output_right: Vec<f32>,
    _input_channel_buffers: Vec<*mut f32>,
    _output_channel_buffers: Vec<*mut f32>,
    _input_buses: Vec<AudioBusBuffers>,
    output_buses: Vec<AudioBusBuffers>,
}

fn stereo_process_fixture(samples: usize, output_value: f32) -> StereoProcessFixture {
    let mut input_left = vec![1.0; samples];
    let mut input_right = vec![0.5; samples];
    let mut output_left = vec![output_value; samples];
    let mut output_right = vec![output_value; samples];
    let mut input_channel_buffers = vec![input_left.as_mut_ptr(), input_right.as_mut_ptr()];
    let mut output_channel_buffers = vec![output_left.as_mut_ptr(), output_right.as_mut_ptr()];

    let input_bus = AudioBusBuffers {
        numChannels: 2,
        silenceFlags: 0,
        __field0: AudioBusBuffers__type0 {
            channelBuffers32: input_channel_buffers.as_mut_ptr(),
        },
    };
    let output_bus = AudioBusBuffers {
        numChannels: 2,
        silenceFlags: 0,
        __field0: AudioBusBuffers__type0 {
            channelBuffers32: output_channel_buffers.as_mut_ptr(),
        },
    };
    let mut input_buses = vec![input_bus];
    let mut output_buses = vec![output_bus];
    let mut process_data: ProcessData = unsafe { mem::zeroed() };
    process_data.symbolicSampleSize = SymbolicSampleSizes_::kSample32 as i32;
    process_data.numSamples = i32::try_from(samples).expect("sample count should fit i32");
    process_data.numInputs = 1;
    process_data.numOutputs = 1;
    process_data.inputs = input_buses.as_mut_ptr();
    process_data.outputs = output_buses.as_mut_ptr();

    StereoProcessFixture {
        process_data,
        _input_left: input_left,
        _input_right: input_right,
        output_left,
        output_right,
        _input_channel_buffers: input_channel_buffers,
        _output_channel_buffers: output_channel_buffers,
        _input_buses: input_buses,
        output_buses,
    }
}

#[test]
fn controller_reports_expected_parameter_count() {
    let controller = PumpVst3Controller::new(Arc::new(PumpVst3Shared::new()));
    let count = unsafe { controller.getParameterCount() };
    assert_eq!(count, 14);
}

#[test]
fn controller_marks_only_appended_bypass_as_stepped_host_bypass() {
    let controller = PumpVst3Controller::new(Arc::new(PumpVst3Shared::new()));
    let mut info: ParameterInfo = unsafe { mem::zeroed() };
    assert_eq!(
        unsafe { controller.getParameterInfo(7, &mut info) },
        kResultOk
    );
    assert_eq!(info.id, crate::params::PARAM_BYPASS_NUM);
    assert_eq!(info.stepCount, 1);
    assert_eq!(info.defaultNormalizedValue, 0.0);
    assert_ne!(
        info.flags & ParameterInfo_::ParameterFlags_::kCanAutomate,
        0
    );
    assert_ne!(info.flags & ParameterInfo_::ParameterFlags_::kIsBypass, 0);

    let mut preceding: ParameterInfo = unsafe { mem::zeroed() };
    assert_eq!(
        unsafe { controller.getParameterInfo(6, &mut preceding) },
        kResultOk
    );
    assert_eq!(preceding.id, crate::params::PARAM_SMOOTH_NUM);
    assert_eq!(
        preceding.flags & ParameterInfo_::ParameterFlags_::kIsBypass,
        0
    );
}

#[cfg(target_os = "macos")]
#[test]
fn vst3_ui_sink_delivers_begin_value_end_on_component_handler() {
    let shared = Arc::new(PumpVst3Shared::new());
    let controller = PumpVst3Controller::new(Arc::clone(&shared));
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let handler = ComWrapper::new(RecordingComponentHandler {
        calls: Arc::clone(&calls),
        begin_result: kResultOk,
        perform_result: kResultOk,
        end_result: kResultOk,
    })
    .to_com_ptr::<IComponentHandler>()
    .expect("component handler interface");
    assert_eq!(
        unsafe { controller.setComponentHandler(handler.as_ptr()) },
        kResultOk
    );

    let sink = gui_adapter::Vst3HostParamEditSink {
        shared: Arc::clone(&shared),
    };
    assert!(crate::gui::try_toggle_bypass(
        shared.params.as_ref(),
        &sink,
        &toybox::clap::automation::AutomationConfig::default(),
    ));
    assert!(shared.params.bypassed());
    assert_eq!(
        *calls.lock().expect("recorded calls lock"),
        vec![
            RecordedEditCall::Begin(crate::params::PARAM_BYPASS_NUM),
            RecordedEditCall::Perform(crate::params::PARAM_BYPASS_NUM, 1.0),
            RecordedEditCall::End(crate::params::PARAM_BYPASS_NUM),
        ]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn vst3_ui_sink_delivers_continuous_begin_value_end_on_component_handler() {
    let shared = Arc::new(PumpVst3Shared::new());
    let controller = PumpVst3Controller::new(Arc::clone(&shared));
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let handler = ComWrapper::new(RecordingComponentHandler {
        calls: Arc::clone(&calls),
        begin_result: kResultOk,
        perform_result: kResultOk,
        end_result: kResultOk,
    })
    .to_com_ptr::<IComponentHandler>()
    .expect("component handler interface");
    assert_eq!(
        unsafe { controller.setComponentHandler(handler.as_ptr()) },
        kResultOk
    );

    let sink = gui_adapter::Vst3HostParamEditSink {
        shared: Arc::clone(&shared),
    };
    let config = toybox::clap::automation::AutomationConfig::default();
    let param_id = crate::params::PARAM_MIX_ID;
    assert!(sink.gesture_started(&config, param_id));
    assert!(sink.gesture_value(&config, param_id, 0.375));
    assert!(sink.gesture_ended(&config, param_id));
    assert_eq!(
        *calls.lock().expect("recorded calls lock"),
        vec![
            RecordedEditCall::Begin(crate::params::PARAM_MIX_NUM),
            RecordedEditCall::Perform(crate::params::PARAM_MIX_NUM, 0.375),
            RecordedEditCall::End(crate::params::PARAM_MIX_NUM),
        ]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn vst3_ui_sink_maps_clap_division_to_extended_vst3_id() {
    let shared = Arc::new(PumpVst3Shared::new());
    let controller = PumpVst3Controller::new(Arc::clone(&shared));
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let handler = ComWrapper::new(RecordingComponentHandler {
        calls: Arc::clone(&calls),
        begin_result: kResultOk,
        perform_result: kResultOk,
        end_result: kResultOk,
    })
    .to_com_ptr::<IComponentHandler>()
    .expect("component handler interface");
    assert_eq!(
        unsafe { controller.setComponentHandler(handler.as_ptr()) },
        kResultOk
    );

    let sink = gui_adapter::Vst3HostParamEditSink {
        shared: Arc::clone(&shared),
    };
    let config = toybox::clap::automation::AutomationConfig::default();
    assert!(sink.edit(&config, PARAM_SYNC_DIVISION_ID, 8.0));
    assert!(sink.edit(&config, PARAM_SYNC_DIVISION_ID, 9.0));

    assert_eq!(
        *calls.lock().expect("recorded calls lock"),
        vec![
            RecordedEditCall::Begin(PARAM_SYNC_DIVISION_VST3_V2_NUM),
            RecordedEditCall::Perform(PARAM_SYNC_DIVISION_VST3_V2_NUM, 8.0 / 9.0),
            RecordedEditCall::End(PARAM_SYNC_DIVISION_VST3_V2_NUM),
            RecordedEditCall::Begin(PARAM_SYNC_DIVISION_VST3_V2_NUM),
            RecordedEditCall::Perform(PARAM_SYNC_DIVISION_VST3_V2_NUM, 1.0),
            RecordedEditCall::End(PARAM_SYNC_DIVISION_VST3_V2_NUM),
        ]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn vst3_ui_sink_keeps_bypass_state_when_component_handler_is_missing() {
    let shared = Arc::new(PumpVst3Shared::new());
    let sink = gui_adapter::Vst3HostParamEditSink {
        shared: Arc::clone(&shared),
    };

    assert!(!crate::gui::try_toggle_bypass(
        shared.params.as_ref(),
        &sink,
        &toybox::clap::automation::AutomationConfig::default(),
    ));
    assert!(!shared.params.bypassed());
}

#[cfg(target_os = "macos")]
#[test]
fn vst3_ui_sink_keeps_bypass_state_when_component_handler_rejects_edit() {
    let shared = Arc::new(PumpVst3Shared::new());
    let controller = PumpVst3Controller::new(Arc::clone(&shared));
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let handler = ComWrapper::new(RecordingComponentHandler {
        calls: Arc::clone(&calls),
        begin_result: kResultOk,
        perform_result: kResultFalse,
        end_result: kResultOk,
    })
    .to_com_ptr::<IComponentHandler>()
    .expect("component handler interface");
    assert_eq!(
        unsafe { controller.setComponentHandler(handler.as_ptr()) },
        kResultOk
    );
    let sink = gui_adapter::Vst3HostParamEditSink {
        shared: Arc::clone(&shared),
    };

    assert!(!crate::gui::try_toggle_bypass(
        shared.params.as_ref(),
        &sink,
        &toybox::clap::automation::AutomationConfig::default(),
    ));
    assert!(!shared.params.bypassed());
    assert_eq!(
        *calls.lock().expect("recorded calls lock"),
        vec![
            RecordedEditCall::Begin(crate::params::PARAM_BYPASS_NUM),
            RecordedEditCall::Perform(crate::params::PARAM_BYPASS_NUM, 1.0),
            RecordedEditCall::End(crate::params::PARAM_BYPASS_NUM),
        ]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn vst3_ui_sink_short_circuits_when_component_handler_rejects_begin() {
    let shared = Arc::new(PumpVst3Shared::new());
    let controller = PumpVst3Controller::new(Arc::clone(&shared));
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let handler = ComWrapper::new(RecordingComponentHandler {
        calls: Arc::clone(&calls),
        begin_result: kResultFalse,
        perform_result: kResultOk,
        end_result: kResultOk,
    })
    .to_com_ptr::<IComponentHandler>()
    .expect("component handler interface");
    assert_eq!(
        unsafe { controller.setComponentHandler(handler.as_ptr()) },
        kResultOk
    );
    let sink = gui_adapter::Vst3HostParamEditSink {
        shared: Arc::clone(&shared),
    };

    assert!(!crate::gui::try_toggle_bypass(
        shared.params.as_ref(),
        &sink,
        &toybox::clap::automation::AutomationConfig::default(),
    ));
    assert!(!shared.params.bypassed());
    assert_eq!(
        *calls.lock().expect("recorded calls lock"),
        vec![RecordedEditCall::Begin(crate::params::PARAM_BYPASS_NUM)]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn vst3_ui_sink_commits_accepted_value_when_component_handler_rejects_end() {
    let shared = Arc::new(PumpVst3Shared::new());
    let controller = PumpVst3Controller::new(Arc::clone(&shared));
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let handler = ComWrapper::new(RecordingComponentHandler {
        calls: Arc::clone(&calls),
        begin_result: kResultOk,
        perform_result: kResultOk,
        end_result: kResultFalse,
    })
    .to_com_ptr::<IComponentHandler>()
    .expect("component handler interface");
    assert_eq!(
        unsafe { controller.setComponentHandler(handler.as_ptr()) },
        kResultOk
    );
    let sink = gui_adapter::Vst3HostParamEditSink {
        shared: Arc::clone(&shared),
    };

    assert!(crate::gui::try_toggle_bypass(
        shared.params.as_ref(),
        &sink,
        &toybox::clap::automation::AutomationConfig::default(),
    ));
    assert!(shared.params.bypassed());
    assert_eq!(
        *calls.lock().expect("recorded calls lock"),
        vec![
            RecordedEditCall::Begin(crate::params::PARAM_BYPASS_NUM),
            RecordedEditCall::Perform(crate::params::PARAM_BYPASS_NUM, 1.0),
            RecordedEditCall::End(crate::params::PARAM_BYPASS_NUM),
        ]
    );
}

#[test]
fn processor_declares_single_stereo_main_bus() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));

    assert_eq!(
        unsafe {
            processor.getBusCount(
                MediaTypes_::kAudio as MediaType,
                BusDirections_::kInput as BusDirection,
            )
        },
        1
    );
    assert_eq!(
        unsafe {
            processor.getBusCount(
                MediaTypes_::kAudio as MediaType,
                BusDirections_::kOutput as BusDirection,
            )
        },
        1
    );

    let mut arrangement = SpeakerArrangement::default();
    assert_eq!(
        unsafe {
            processor.getBusArrangement(BusDirections_::kInput as BusDirection, 0, &mut arrangement)
        },
        kResultOk
    );
    assert_eq!(arrangement, SpeakerArr::kStereo);
}

#[test]
fn processor_accepts_input_presentation_latency_and_ignores_output_latency() {
    let shared = Arc::new(PumpVst3Shared::new());
    let processor = PumpVst3Processor::new(Arc::clone(&shared));

    assert_eq!(unsafe { processor.getLatencySamples() }, 0);
    assert_eq!(
        unsafe {
            processor.setAudioPresentationLatencySamples(
                BusDirections_::kInput as BusDirection,
                0,
                512,
            )
        },
        kResultOk
    );
    assert_eq!(
        processor
            .runtime_handoff
            .input_presentation_latency_samples(),
        512
    );

    assert_eq!(
        unsafe {
            processor.setAudioPresentationLatencySamples(
                BusDirections_::kOutput as BusDirection,
                0,
                1024,
            )
        },
        kResultOk
    );
    assert_eq!(
        processor
            .runtime_handoff
            .input_presentation_latency_samples(),
        512
    );
}

#[test]
fn processor_rejects_invalid_presentation_latency_buses_without_mutation() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));
    processor
        .runtime_handoff
        .publish_input_presentation_latency(512);

    for (direction, bus_index) in [
        (BusDirections_::kInput as BusDirection, 1),
        (BusDirections_::kOutput as BusDirection, 1),
        (999, 0),
    ] {
        assert_eq!(
            unsafe { processor.setAudioPresentationLatencySamples(direction, bus_index, 1024) },
            kInvalidArgument
        );
        assert_eq!(
            processor
                .runtime_handoff
                .input_presentation_latency_samples(),
            512
        );
    }
}

#[test]
fn processor_resets_input_presentation_latency_when_deactivated() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));
    processor
        .runtime_handoff
        .publish_input_presentation_latency(512);

    assert_eq!(unsafe { processor.setActive(0) }, kResultOk);
    assert_eq!(
        processor
            .runtime_handoff
            .input_presentation_latency_samples(),
        0
    );
}

#[test]
fn active_lifecycle_invalidates_waveform_without_process_and_republishes_after_restart() {
    let shared = Arc::new(PumpVst3Shared::new());
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut fixture = stereo_process_fixture(64, 9.0);

    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert!(shared.status.incoming_waveform_snapshot().is_some());

    assert_eq!(unsafe { processor.setActive(0) }, kResultOk);
    assert!(
        shared.status.incoming_waveform_snapshot().is_none(),
        "deactivation must hide the old waveform before another process callback"
    );
    assert_eq!(unsafe { processor.setActive(1) }, kResultOk);
    assert!(
        shared.status.incoming_waveform_snapshot().is_none(),
        "reactivation must not expose the old waveform"
    );

    let mut after_restart = stereo_process_fixture(64, 9.0);
    assert_eq!(
        unsafe { processor.process(&mut after_restart.process_data) },
        process_ok()
    );
    assert!(
        shared.status.incoming_waveform_snapshot().is_some(),
        "the next audio callback must republish a fresh waveform"
    );
}

#[test]
#[cfg(target_os = "macos")]
#[allow(dead_code)]
fn controller_creates_editor_view_for_host_editor_request() {
    let controller = PumpVst3Controller::new(Arc::new(PumpVst3Shared::new()));
    let view = unsafe { controller.createView(ViewType::kEditor) };
    assert!(!view.is_null(), "editor view should be creatable");
    unsafe {
        let unknown = view.cast::<FUnknown>();
        ((*(*unknown).vtbl).release)(unknown);
    }
}

#[test]
#[cfg(not(target_os = "macos"))]
fn controller_does_not_advertise_editor_view_off_macos() {
    let controller = PumpVst3Controller::new(Arc::new(PumpVst3Shared::new()));
    let view = unsafe { controller.createView(ViewType::kEditor) };
    assert!(view.is_null(), "editor view is macOS-only");
}

#[test]
#[cfg(target_os = "macos")]
fn view_enforces_minimum_size() {
    let (preferred_width, preferred_height) = preferred_window_size();
    let view = HostedVst3View::new(
        PumpVst3GuiAdapter::new(Arc::new(PumpVst3Shared::new())),
        preferred_width,
        preferred_height,
    )
    .with_size_bounds(
        MIN_WINDOW_WIDTH,
        MIN_WINDOW_HEIGHT,
        MAX_WINDOW_WIDTH,
        MAX_WINDOW_HEIGHT,
    );
    let mut rect = view_rect(10, 10);
    let result = unsafe { view.checkSizeConstraint(&mut rect) };
    assert_eq!(result, kResultOk);
    assert_eq!(rect.right - rect.left, MIN_WINDOW_WIDTH as i32);
    assert_eq!(rect.bottom - rect.top, MIN_WINDOW_HEIGHT as i32);
}

#[test]
#[cfg(target_os = "macos")]
fn view_reports_default_size_and_clamps_supported_maximum() {
    let (preferred_width, preferred_height) = preferred_window_size();
    let view = HostedVst3View::new(
        PumpVst3GuiAdapter::new(Arc::new(PumpVst3Shared::new())),
        preferred_width,
        preferred_height,
    )
    .with_size_bounds(
        MIN_WINDOW_WIDTH,
        MIN_WINDOW_HEIGHT,
        MAX_WINDOW_WIDTH,
        MAX_WINDOW_HEIGHT,
    );

    let mut size = view_rect(0, 0);
    assert_eq!(unsafe { view.getSize(&mut size) }, kResultOk);
    assert_eq!(size.right - size.left, crate::gui::WINDOW_WIDTH as i32);
    assert_eq!(size.bottom - size.top, crate::gui::WINDOW_HEIGHT as i32);

    let mut max_size = view_rect(4_000, 4_000);
    assert_eq!(unsafe { view.onSize(&mut max_size) }, kResultOk);
    assert_eq!(max_size.right - max_size.left, MAX_WINDOW_WIDTH as i32);
    assert_eq!(max_size.bottom - max_size.top, MAX_WINDOW_HEIGHT as i32);
}

#[test]
#[cfg(target_os = "macos")]
fn view_normalizes_off_aspect_host_resize_and_preserves_origin() {
    let (preferred_width, preferred_height) = preferred_window_size();
    let view = HostedVst3View::new(
        PumpVst3GuiAdapter::new(Arc::new(PumpVst3Shared::new())),
        preferred_width,
        preferred_height,
    )
    .with_size_bounds(
        MIN_WINDOW_WIDTH,
        MIN_WINDOW_HEIGHT,
        MAX_WINDOW_WIDTH,
        MAX_WINDOW_HEIGHT,
    );

    let mut rect = ViewRect {
        left: 12,
        top: 18,
        right: 1_212,
        bottom: 618,
    };
    assert_eq!(unsafe { view.checkSizeConstraint(&mut rect) }, kResultOk);
    assert_eq!((rect.left, rect.top), (12, 18));
    assert_eq!(rect.right - rect.left, 1_200);
    assert_eq!(rect.bottom - rect.top, 750);
}

#[test]
#[cfg(target_os = "macos")]
fn vst3_gui_adapter_forwards_normalized_resize_to_host_window() {
    let adapter = PumpVst3GuiAdapter::new(Arc::new(PumpVst3Shared::new()));
    adapter.request_resize(MAX_WINDOW_WIDTH * 2, MIN_WINDOW_HEIGHT);
    assert_eq!(
        adapter.last_size(),
        Some((MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT))
    );
}

#[test]
fn controller_and_processor_share_param_state() {
    let shared = Arc::new(PumpVst3Shared::new());
    let controller = PumpVst3Controller::new(Arc::clone(&shared));
    let _processor = PumpVst3Processor::new(Arc::clone(&shared));
    let value = to_normalized(PARAM_MIX_NUM, 0.25);
    let result = unsafe { controller.setParamNormalized(PARAM_MIX_NUM, value) };
    assert_eq!(result, kResultOk);
    assert!((shared.params.mix() - 0.25).abs() < 1.0e-6);
}

#[test]
fn controller_legacy_and_extended_division_ids_update_shared_state() {
    let shared = Arc::new(PumpVst3Shared::new());
    let controller = PumpVst3Controller::new(Arc::clone(&shared));

    assert_eq!(
        unsafe {
            controller.setParamNormalized(
                PARAM_SYNC_DIVISION_NUM,
                to_normalized(PARAM_SYNC_DIVISION_NUM, 7.0),
            )
        },
        kResultOk
    );
    assert_eq!(shared.params.sync_division(), 7);

    assert_eq!(
        unsafe {
            controller.setParamNormalized(
                PARAM_SYNC_DIVISION_VST3_V2_NUM,
                to_normalized(PARAM_SYNC_DIVISION_VST3_V2_NUM, 8.0),
            )
        },
        kResultOk
    );
    assert_eq!(shared.params.sync_division(), 8);
}

#[test]
fn component_and_controller_restore_v14_state_with_sync_timing_defaults() {
    let source = PumpParams::new();
    source.set_timing_mode(TIMING_MODE_FREE as f32);
    source.set_free_rate_hz(17.0);
    let payload = crate::params::payload_for_state_version(&source, 14);

    let processor_shared = Arc::new(PumpVst3Shared::new());
    processor_shared
        .params
        .set_timing_mode(TIMING_MODE_FREE as f32);
    processor_shared.params.set_free_rate_hz(17.0);
    let processor = PumpVst3Processor::new(Arc::clone(&processor_shared));
    let mut processor_stream = TestVst3Stream::v14(payload.clone());
    assert_eq!(
        unsafe { processor.setState(processor_stream.as_ptr()) },
        kResultOk
    );
    assert_eq!(processor_shared.params.timing_mode(), TIMING_MODE_SYNC);
    assert_eq!(processor_shared.params.free_rate_hz(), DEFAULT_FREE_RATE_HZ);

    let controller_shared = Arc::new(PumpVst3Shared::new());
    controller_shared
        .params
        .set_timing_mode(TIMING_MODE_FREE as f32);
    controller_shared.params.set_free_rate_hz(17.0);
    let controller = PumpVst3Controller::new(Arc::clone(&controller_shared));
    let mut controller_stream = TestVst3Stream::v14(payload);
    assert_eq!(
        unsafe { controller.setComponentState(controller_stream.as_ptr()) },
        kResultOk
    );
    assert_eq!(controller_shared.params.timing_mode(), TIMING_MODE_SYNC);
    assert_eq!(
        controller_shared.params.free_rate_hz(),
        DEFAULT_FREE_RATE_HZ
    );
}

#[test]
fn processor_writes_the_full_normal_output_range() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));
    let mut fixture = stereo_process_fixture(64, 9.0);

    let result = unsafe { processor.process(&mut fixture.process_data) };

    assert_eq!(result, process_ok());
    assert!(fixture.output_left.iter().all(|sample| sample.is_finite()));
    assert!(fixture.output_right.iter().all(|sample| sample.is_finite()));
    assert!(fixture.output_left.iter().all(|sample| *sample != 9.0));
    assert!(fixture.output_right.iter().all(|sample| *sample != 9.0));
}

#[test]
fn processor_applies_extended_division_in_sample_and_zero_frame_paths() {
    let shared = Arc::new(PumpVst3Shared::new());
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut fixture = stereo_process_fixture(2, 9.0);
    let mut sample_changes = TestParameterChanges::new(vec![(
        PARAM_SYNC_DIVISION_VST3_V2_NUM,
        vec![(1, to_normalized(PARAM_SYNC_DIVISION_VST3_V2_NUM, 8.0))],
    )]);
    fixture.process_data.inputParameterChanges = sample_changes.as_ptr();

    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert_eq!(shared.params.sync_division(), 8);

    let mut zero_frame = stereo_process_fixture(0, 9.0);
    let mut zero_frame_changes = TestParameterChanges::new(vec![(
        PARAM_SYNC_DIVISION_VST3_V2_NUM,
        vec![(0, to_normalized(PARAM_SYNC_DIVISION_VST3_V2_NUM, 9.0))],
    )]);
    zero_frame.process_data.symbolicSampleSize = SymbolicSampleSizes_::kSample64 as i32;
    zero_frame.process_data.inputParameterChanges = zero_frame_changes.as_ptr();

    assert_eq!(
        unsafe { processor.process(&mut zero_frame.process_data) },
        process_ok()
    );
    assert_eq!(shared.params.sync_division(), 9);
}

#[test]
fn processor_shows_the_initial_input_waveform_and_clears_it_when_input_disappears() {
    let shared = Arc::new(PumpVst3Shared::new());
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut fixture = stereo_process_fixture(64, 9.0);

    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    let snapshot = shared
        .status
        .incoming_waveform_snapshot()
        .expect("the first cycle should be visible while it is captured");
    assert!(snapshot.iter().copied().fold(0.0_f32, f32::max) >= 1.0);

    fixture.process_data.inputs = ptr::null_mut();
    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert!(shared.status.incoming_waveform_snapshot().is_none());
}

#[test]
fn empty_vst3_blocks_do_not_refresh_a_stale_waveform() {
    let shared = Arc::new(PumpVst3Shared::new());
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut fixture = stereo_process_fixture(64, 9.0);

    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert!(shared.status.incoming_waveform_snapshot().is_some());

    shared
        .status
        .incoming_waveform_buffer()
        .set_last_update_micros_for_test(0);
    fixture.process_data.numSamples = 0;
    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );

    assert!(
        shared.status.incoming_waveform_snapshot().is_none(),
        "an empty keepalive block must not republish or refresh the old input peak"
    );
}

#[test]
fn processing_reset_clears_stale_waveform_state() {
    let shared = Arc::new(PumpVst3Shared::new());
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut fixture = stereo_process_fixture(64, 9.0);

    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert!(shared.status.incoming_waveform_snapshot().is_some());

    assert_eq!(unsafe { processor.setProcessing(0) }, kResultOk);
    assert!(
        shared.status.incoming_waveform_snapshot().is_none(),
        "setProcessing(0) must hide the old waveform immediately"
    );
    assert_eq!(unsafe { processor.setProcessing(1) }, kResultOk);
    assert!(
        shared.status.incoming_waveform_snapshot().is_none(),
        "setProcessing(1) must keep the old waveform hidden"
    );

    fixture = stereo_process_fixture(64, 9.0);
    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert!(
        shared.status.incoming_waveform_snapshot().is_some(),
        "the next audio callback must republish a fresh waveform"
    );
}

#[test]
fn empty_free_vst3_block_preserves_last_dsp_phase_and_applied_offset() {
    let shared = Arc::new(PumpVst3Shared::new());
    shared.params.set_timing_mode(TIMING_MODE_FREE as f32);
    shared.params.set_free_rate_hz(2.5);
    shared.params.set_phase_offset(0.7);
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut fixture = stereo_process_fixture(64, 9.0);

    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    let before = shared
        .status
        .dsp_snapshot()
        .expect("a non-empty block must publish DSP telemetry");
    assert!(before.applied_phase_offset > 0.0);
    assert!(before.applied_phase_offset < 0.7);

    fixture.process_data.numSamples = 0;
    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );

    assert_eq!(
        shared.status.dsp_snapshot(),
        Some(before),
        "an empty Free-mode callback must preserve the last valid DSP pair"
    );
}

#[test]
fn zero_sample_parameter_flush_without_buses_applies_all_points_and_preserves_state() {
    let shared = Arc::new(PumpVst3Shared::new());
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut initial = stereo_process_fixture(64, 9.0);
    assert_eq!(
        unsafe { processor.process(&mut initial.process_data) },
        process_ok()
    );

    let generation_before = shared
        .status
        .incoming_waveform_buffer()
        .generation_for_test();
    let dsp_before = shared.status.dsp_snapshot();
    shared
        .status
        .incoming_waveform_buffer()
        .set_last_update_micros_for_test(1234);

    let mut changes = TestParameterChanges::new(vec![
        (
            PARAM_PHASE_OFFSET_NUM,
            vec![
                (0, to_normalized(PARAM_PHASE_OFFSET_NUM, 0.2)),
                (12, to_normalized(PARAM_PHASE_OFFSET_NUM, 0.4)),
            ],
        ),
        (
            PARAM_FREE_RATE_NUM,
            vec![(7, to_normalized(PARAM_FREE_RATE_NUM, 17.0))],
        ),
    ]);
    let mut process_data: ProcessData = unsafe { mem::zeroed() };
    process_data.numSamples = 0;
    process_data.symbolicSampleSize = SymbolicSampleSizes_::kSample64 as i32;
    process_data.inputParameterChanges = changes.as_ptr();

    assert_eq!(
        unsafe { processor.process(&mut process_data) },
        process_ok()
    );
    assert!((shared.params.phase_offset() - 0.4).abs() < 1.0e-6);
    assert!((shared.params.free_rate_hz() - 17.0).abs() < 1.0e-5);
    assert_eq!(
        shared
            .status
            .incoming_waveform_buffer()
            .generation_for_test(),
        generation_before,
        "offset and Free-rate-only flushes retain the current generation"
    );
    assert_eq!(
        shared
            .status
            .incoming_waveform_buffer()
            .last_update_micros_for_test(),
        1234,
        "a non-mapping flush must not refresh the waveform timestamp"
    );
    assert_eq!(shared.status.dsp_snapshot(), dsp_before);
}

#[test]
fn zero_sample_parameter_flush_with_empty_stereo_buffers_preserves_host_state() {
    let shared = Arc::new(PumpVst3Shared::new());
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut initial = stereo_process_fixture(64, 9.0);
    assert_eq!(
        unsafe { processor.process(&mut initial.process_data) },
        process_ok()
    );

    let generation_before = shared
        .status
        .incoming_waveform_buffer()
        .generation_for_test();
    shared
        .status
        .incoming_waveform_buffer()
        .set_last_update_micros_for_test(2345);
    let dsp_before = shared.status.dsp_snapshot();

    let mut fixture = stereo_process_fixture(0, 9.0);
    fixture._input_buses[0].silenceFlags = 0x11;
    fixture.output_buses[0].silenceFlags = 0x22;
    let mut changes = TestParameterChanges::new(vec![
        (
            PARAM_PHASE_OFFSET_NUM,
            vec![(0, to_normalized(PARAM_PHASE_OFFSET_NUM, 0.3))],
        ),
        (
            PARAM_FREE_RATE_NUM,
            vec![(0, to_normalized(PARAM_FREE_RATE_NUM, 9.0))],
        ),
    ]);
    fixture.process_data.symbolicSampleSize = SymbolicSampleSizes_::kSample64 as i32;
    fixture.process_data.inputParameterChanges = changes.as_ptr();

    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert_eq!(fixture._input_buses[0].silenceFlags, 0x11);
    assert_eq!(fixture.output_buses[0].silenceFlags, 0x22);
    assert!((shared.params.phase_offset() - 0.3).abs() < 1.0e-6);
    assert!((shared.params.free_rate_hz() - 9.0).abs() < 1.0e-5);
    assert_eq!(
        shared
            .status
            .incoming_waveform_buffer()
            .generation_for_test(),
        generation_before
    );
    assert_eq!(
        shared
            .status
            .incoming_waveform_buffer()
            .last_update_micros_for_test(),
        2345
    );
    assert_eq!(shared.status.dsp_snapshot(), dsp_before);
}

#[test]
fn zero_sample_parameter_flush_with_declared_null_buffers_reconciles_mapping_once() {
    let shared = Arc::new(PumpVst3Shared::new());
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut initial = stereo_process_fixture(64, 9.0);
    assert_eq!(
        unsafe { processor.process(&mut initial.process_data) },
        process_ok()
    );

    let generation_before = shared
        .status
        .incoming_waveform_buffer()
        .generation_for_test();
    shared
        .status
        .incoming_waveform_buffer()
        .set_last_update_micros_for_test(3456);
    let dsp_before = shared.status.dsp_snapshot();

    let mut fixture = stereo_process_fixture(0, 9.0);
    fixture._input_buses[0].silenceFlags = 0x33;
    fixture.output_buses[0].silenceFlags = 0x44;
    fixture._input_buses[0].__field0.channelBuffers32 = ptr::null_mut();
    fixture.output_buses[0].__field0.channelBuffers32 = ptr::null_mut();
    let mut changes = TestParameterChanges::new(vec![
        (
            PARAM_TIMING_MODE_NUM,
            vec![(
                0,
                to_normalized(PARAM_TIMING_MODE_NUM, TIMING_MODE_FREE as f64),
            )],
        ),
        (
            PARAM_SYNC_DIVISION_NUM,
            vec![(0, to_normalized(PARAM_SYNC_DIVISION_NUM, 6.0))],
        ),
        (
            PARAM_SWING_NUM,
            vec![(0, to_normalized(PARAM_SWING_NUM, 0.35))],
        ),
    ]);
    fixture.process_data.symbolicSampleSize = SymbolicSampleSizes_::kSample64 as i32;
    fixture.process_data.inputParameterChanges = changes.as_ptr();

    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert_eq!(shared.params.timing_mode(), TIMING_MODE_FREE);
    assert_eq!(shared.params.sync_division(), 6);
    assert!((shared.params.swing() - 0.35).abs() < 1.0e-6);
    let generation_after_mapping_change = generation_before.wrapping_add(1);
    assert_eq!(
        shared
            .status
            .incoming_waveform_buffer()
            .generation_for_test(),
        generation_after_mapping_change
    );
    assert_eq!(
        shared
            .status
            .incoming_waveform_buffer()
            .last_update_micros_for_test(),
        0
    );
    assert!(shared.status.incoming_waveform_snapshot().is_none());
    assert_eq!(shared.status.dsp_snapshot(), dsp_before);
    assert_eq!(fixture._input_buses[0].silenceFlags, 0x33);
    assert_eq!(fixture.output_buses[0].silenceFlags, 0x44);

    let mut first_real_sample = stereo_process_fixture(1, 9.0);
    assert_eq!(
        unsafe { processor.process(&mut first_real_sample.process_data) },
        process_ok()
    );
    assert_eq!(
        shared
            .status
            .incoming_waveform_buffer()
            .generation_for_test(),
        generation_after_mapping_change,
        "the first real sample must not invalidate the zero-frame transition again"
    );
    assert!(
        shared.status.incoming_waveform_snapshot().is_none(),
        "the first real block remains a fresh hidden capture"
    );
}

#[test]
fn silent_vst3_blocks_do_not_refresh_a_stale_waveform() {
    let shared = Arc::new(PumpVst3Shared::new());
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut fixture = stereo_process_fixture(64, 9.0);

    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert!(shared.status.incoming_waveform_snapshot().is_some());

    shared
        .status
        .incoming_waveform_buffer()
        .set_last_update_micros_for_test(0);
    fixture._input_left.fill(0.0);
    fixture._input_right.fill(0.0);
    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );

    assert!(
        shared.status.incoming_waveform_snapshot().is_none(),
        "an all-silent block must not republish or refresh the old input peak"
    );
}

#[test]
fn processor_supports_exact_in_place_channel_buffers() {
    let shared = Arc::new(PumpVst3Shared::new());
    shared.params.set_output_gain_db(-24.0);
    let processor = PumpVst3Processor::new(shared);
    let mut fixture = stereo_process_fixture(64, 9.0);
    fixture._output_channel_buffers[0] = fixture._input_left.as_mut_ptr();
    fixture._output_channel_buffers[1] = fixture._input_right.as_mut_ptr();

    let result = unsafe { processor.process(&mut fixture.process_data) };

    assert_eq!(result, process_ok());
    assert!(fixture._input_left.iter().all(|sample| sample.is_finite()));
    assert!(fixture._input_right.iter().all(|sample| sample.is_finite()));
    assert!(fixture._input_left.iter().any(|sample| *sample < 1.0));
    assert!(fixture._input_right.iter().any(|sample| *sample < 0.5));
    assert_eq!(fixture.output_buses[0].silenceFlags, 0);
}

#[test]
fn processor_silences_valid_outputs_when_input_is_missing() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));
    let mut fixture = stereo_process_fixture(32, 9.0);
    fixture.process_data.inputs = ptr::null_mut();

    let result = unsafe { processor.process(&mut fixture.process_data) };

    assert_eq!(result, process_ok());
    assert_eq!(fixture.output_left, vec![0.0; 32]);
    assert_eq!(fixture.output_right, vec![0.0; 32]);
    assert_eq!(fixture.output_buses[0].silenceFlags, 0b11);
}

#[test]
fn processor_clears_reused_output_silence_flags_after_normal_processing() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));
    let mut fixture = stereo_process_fixture(32, 9.0);
    fixture.process_data.inputs = ptr::null_mut();
    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert_eq!(fixture.output_buses[0].silenceFlags, 0b11);

    fixture.process_data.inputs = fixture._input_buses.as_mut_ptr();
    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );

    assert_eq!(fixture.output_buses[0].silenceFlags, 0);
    assert!(fixture.output_left.iter().any(|sample| *sample != 0.0));
    assert!(fixture.output_right.iter().any(|sample| *sample != 0.0));
}

#[test]
fn processor_rejects_unwritable_output_instead_of_claiming_success() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));
    let mut fixture = stereo_process_fixture(32, 9.0);
    fixture.process_data.outputs = ptr::null_mut();

    let result = unsafe { processor.process(&mut fixture.process_data) };

    assert_eq!(result, kInvalidArgument);
}

#[test]
fn processor_rejects_unsupported_sample_size_without_touching_output() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));
    let mut fixture = stereo_process_fixture(32, 9.0);
    fixture.process_data.symbolicSampleSize = SymbolicSampleSizes_::kSample64 as i32;

    let result = unsafe { processor.process(&mut fixture.process_data) };

    assert_eq!(result, kInvalidArgument);
    assert_eq!(fixture.output_left, vec![9.0; 32]);
    assert_eq!(fixture.output_right, vec![9.0; 32]);
}

#[test]
fn processor_rejects_negative_sample_count_instead_of_treating_it_as_zero() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));
    let mut fixture = stereo_process_fixture(1, 9.0);
    fixture.process_data.numSamples = -1;

    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        kInvalidArgument
    );
    assert_eq!(fixture.output_left, vec![9.0]);
    assert_eq!(fixture.output_right, vec![9.0]);
}

#[test]
fn processor_rejects_negative_sample_count_without_outputs_before_applying_parameters() {
    let shared = Arc::new(PumpVst3Shared::new());
    shared.params.set_phase_offset(0.2);
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut changes = TestParameterChanges::new(vec![(
        PARAM_PHASE_OFFSET_NUM,
        vec![(0, to_normalized(PARAM_PHASE_OFFSET_NUM, 0.8))],
    )]);
    let mut process_data: ProcessData = unsafe { mem::zeroed() };
    process_data.numSamples = -1;
    process_data.numOutputs = 0;
    process_data.inputParameterChanges = changes.as_ptr();

    assert_eq!(
        unsafe { processor.process(&mut process_data) },
        kInvalidArgument
    );
    assert_eq!(shared.params.phase_offset(), 0.2);
}

#[test]
fn processor_accepts_an_empty_no_buffer_block() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));
    let mut process_data: ProcessData = unsafe { mem::zeroed() };

    let result = unsafe { processor.process(&mut process_data) };

    assert_eq!(result, process_ok());
}

#[test]
fn processor_accepts_a_positive_length_zero_bus_parameter_flush() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));
    let mut process_data: ProcessData = unsafe { mem::zeroed() };
    process_data.numSamples = 64;
    process_data.symbolicSampleSize = SymbolicSampleSizes_::kSample64 as i32;

    let result = unsafe { processor.process(&mut process_data) };

    assert_eq!(result, process_ok());
}

#[test]
fn processor_accepts_an_omitted_deactivated_output_bus() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));
    let mut fixture = stereo_process_fixture(64, 9.0);
    fixture.process_data.numOutputs = 0;
    fixture.process_data.outputs = ptr::null_mut();
    fixture.process_data.symbolicSampleSize = SymbolicSampleSizes_::kSample64 as i32;

    let result = unsafe { processor.process(&mut fixture.process_data) };

    assert_eq!(result, process_ok());
    assert_eq!(fixture.output_left, vec![9.0; 64]);
    assert_eq!(fixture.output_right, vec![9.0; 64]);
}

#[test]
fn processing_lifecycle_reset_restarts_smoothing_from_unity() {
    let shared = Arc::new(PumpVst3Shared::new());
    shared.params.set_mix(1.0);
    shared.params.set_smooth(1.0);
    shared
        .params
        .set_curve(&[0.0; crate::curve::CURVE_TABLE_LEN]);
    let processor = PumpVst3Processor::new(Arc::clone(&shared));

    let mut driven = stereo_process_fixture(48_000, 1.0);
    assert_eq!(
        unsafe { processor.process(&mut driven.process_data) },
        process_ok()
    );
    assert!(driven.output_left[47_999] < 0.1);

    assert_eq!(unsafe { processor.setProcessing(1) }, kResultOk);
    let mut after_reset = stereo_process_fixture(1, 1.0);
    assert_eq!(
        unsafe { processor.process(&mut after_reset.process_data) },
        process_ok()
    );
    assert!(after_reset.output_left[0] > 0.9);
}

#[test]
fn processor_silences_instead_of_waiting_for_reentrant_runtime_access() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));
    let _runtime_guard = processor
        .runtime
        .try_acquire()
        .expect("test should acquire the idle runtime");
    let mut fixture = stereo_process_fixture(32, 9.0);

    let result = unsafe { processor.process(&mut fixture.process_data) };

    assert_eq!(result, process_ok());
    assert_eq!(fixture.output_left, vec![0.0; 32]);
    assert_eq!(fixture.output_right, vec![0.0; 32]);
    assert_eq!(fixture.output_buses[0].silenceFlags, 0b11);
}

#[test]
fn setup_and_state_handoffs_remain_nonblocking_during_processing() {
    let shared = Arc::new(PumpVst3Shared::new());
    let processor = Arc::new(PumpVst3Processor::new(Arc::clone(&shared)));
    let producer = Arc::clone(&processor);
    let producer_thread = std::thread::spawn(move || {
        for sample_rate in [44_100.0, 48_000.0, 88_200.0, 96_000.0].repeat(64) {
            let mut setup: ProcessSetup = unsafe { mem::zeroed() };
            setup.sampleRate = sample_rate;
            assert_eq!(unsafe { producer.setupProcessing(&mut setup) }, kResultOk);
            producer.runtime_handoff.publish_state_restore();
        }
    });

    let mut fixture = stereo_process_fixture(16, 9.0);
    for _ in 0..256 {
        assert_eq!(
            unsafe { processor.process(&mut fixture.process_data) },
            process_ok()
        );
    }
    producer_thread
        .join()
        .expect("handoff producer should finish");

    let mut final_setup: ProcessSetup = unsafe { mem::zeroed() };
    final_setup.sampleRate = 96_000.0;
    assert_eq!(
        unsafe { processor.setupProcessing(&mut final_setup) },
        kResultOk
    );
    processor.runtime_handoff.publish_state_restore();
    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );

    let runtime = processor
        .runtime
        .try_acquire()
        .expect("runtime should be idle after processing");
    assert!((runtime.sample_rate - 96_000.0).abs() < f32::EPSILON);
    assert_eq!(runtime.last_curve_revision, shared.params.curve_revision());
}

#[test]
fn transport_state_uses_vst3_process_context_when_available() {
    let context = ProcessContext {
        state: (ProcessContext_::StatesAndFlags_::kTempoValid
            | ProcessContext_::StatesAndFlags_::kProjectTimeMusicValid
            | ProcessContext_::StatesAndFlags_::kPlaying),
        sampleRate: 48_000.0,
        projectTimeSamples: 0,
        systemTime: 0,
        continousTimeSamples: 0,
        projectTimeMusic: 17.5,
        barPositionMusic: 0.0,
        cycleStartMusic: 0.0,
        cycleEndMusic: 0.0,
        tempo: 128.0,
        timeSigNumerator: 4,
        timeSigDenominator: 4,
        chord: unsafe { mem::zeroed() },
        smpteOffsetSubframes: 0,
        frameRate: unsafe { mem::zeroed() },
        samplesToNextClock: 0,
    };
    let state =
        transport_state_from_vst3_process_context(&context as *const ProcessContext as *mut _);
    assert!(state.is_playing);
    assert!((state.tempo_bpm - 128.0).abs() < 1.0e-6);
    assert!((state.song_pos_beats.unwrap_or_default() - 17.5).abs() < 1.0e-6);
}

#[test]
fn transport_state_defaults_without_process_context() {
    let state = transport_state_from_vst3_process_context(std::ptr::null_mut());
    assert!(!state.is_playing);
    assert_eq!(state.song_pos_beats, None);
    assert!((state.tempo_bpm - 120.0).abs() < 1.0e-6);
}

#[test]
#[cfg(target_os = "macos")]
fn key_char_prefers_char16_and_falls_back_to_key_code() {
    use toybox::vst3::prelude::Steinberg::VirtualKeyCodes_::{
        KEY_BACK, KEY_END, KEY_ESCAPE, KEY_LEFT, KEY_RETURN,
    };

    assert_eq!(PumpVst3GuiAdapter::key_char('A' as u16, 0), Some('A'));
    assert_eq!(
        PumpVst3GuiAdapter::key_char(0, KEY_BACK as i16),
        Some('\u{8}')
    );
    assert_eq!(
        PumpVst3GuiAdapter::key_char(0, KEY_RETURN as i16),
        Some('\r')
    );
    assert_eq!(
        PumpVst3GuiAdapter::key_char(0, KEY_ESCAPE as i16),
        Some('\u{1b}')
    );
    assert_eq!(
        PumpVst3GuiAdapter::key_char(0, KEY_LEFT as i16),
        Some('\u{1c}')
    );
    assert_eq!(
        PumpVst3GuiAdapter::key_char(0, KEY_END as i16),
        Some('\u{1f}')
    );
    assert_eq!(PumpVst3GuiAdapter::key_char(0, 0x51), Some('Q'));
    assert_eq!(PumpVst3GuiAdapter::key_char(0, 0), None);
}

#[test]
#[cfg(target_os = "macos")]
fn shortcut_modifiers_decode_vst3_bits() {
    let modifiers = PumpVst3GuiAdapter::shortcut_modifiers(0b1001);
    assert!(modifiers.shift);
    assert!(!modifiers.alt);
    assert!(modifiers.ctrl);
}

#[test]
fn release_shared_for_role_clears_controller_claim_only() {
    let shared = Arc::new(PumpVst3Shared::new());
    {
        let mut registry = shared_registry().lock().expect("registry lock");
        registry.push(SharedRegistryEntry {
            shared: Arc::downgrade(&shared),
            processor_claimed: true,
            controller_claimed: true,
        });
    }

    release_shared_for_role(&shared, SharedRole::Controller);

    let registry = shared_registry().lock().expect("registry lock");
    let entry = registry
        .iter()
        .find(|entry| {
            entry
                .shared
                .upgrade()
                .map(|candidate| Arc::ptr_eq(&candidate, &shared))
                .unwrap_or(false)
        })
        .expect("shared entry should exist");
    assert!(entry.processor_claimed);
    assert!(!entry.controller_claimed);
    drop(registry);

    let mut registry = shared_registry().lock().expect("registry lock");
    registry.retain(|entry| {
        !entry
            .shared
            .upgrade()
            .map(|candidate| Arc::ptr_eq(&candidate, &shared))
            .unwrap_or(false)
    });
}
