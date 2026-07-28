use super::*;
use std::mem;
use std::ptr;
use std::sync::Mutex as StdMutex;

#[derive(Clone, Debug, PartialEq)]
enum RecordedEditCall {
    Begin(ParamID),
    Perform(ParamID, ParamValue),
    End(ParamID),
}

struct RecordingComponentHandler {
    calls: Arc<StdMutex<Vec<RecordedEditCall>>>,
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
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        self.calls
            .lock()
            .expect("recording handler lock")
            .push(RecordedEditCall::Perform(id, value));
        kResultOk
    }

    unsafe fn endEdit(&self, id: ParamID) -> tresult {
        self.calls
            .lock()
            .expect("recording handler lock")
            .push(RecordedEditCall::End(id));
        kResultOk
    }

    unsafe fn restartComponent(&self, _flags: i32) -> tresult {
        kResultOk
    }
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
    assert_eq!(count, 10);
}

#[test]
fn controller_marks_only_appended_bypass_as_stepped_host_bypass() {
    let controller = PumpVst3Controller::new(Arc::new(PumpVst3Shared::new()));
    let mut info: ParameterInfo = unsafe { mem::zeroed() };
    assert_eq!(
        unsafe { controller.getParameterInfo(9, &mut info) },
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
        unsafe { controller.getParameterInfo(8, &mut preceding) },
        kResultOk
    );
    assert_eq!(preceding.id, crate::params::PARAM_MODE_NUM);
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
    })
    .to_com_ptr::<IComponentHandler>()
    .expect("component handler interface");
    assert_eq!(
        unsafe { controller.setComponentHandler(handler.as_ptr()) },
        kResultOk
    );

    let sink = gui_adapter::Vst3HostParamEditSink { shared };
    assert!(crate::gui::HostParamEditSink::edit(
        &sink,
        &toybox::clap::automation::AutomationConfig::default(),
        crate::params::PARAM_BYPASS_ID,
        1.0,
    ));
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
fn processor_declares_optional_stereo_sidechain_bus() {
    let processor = PumpVst3Processor::new(Arc::new(PumpVst3Shared::new()));

    assert_eq!(
        unsafe {
            processor.getBusCount(
                MediaTypes_::kAudio as MediaType,
                BusDirections_::kInput as BusDirection,
            )
        },
        2
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

    let mut sidechain_info: BusInfo = unsafe { mem::zeroed() };
    assert_eq!(
        unsafe {
            processor.getBusInfo(
                MediaTypes_::kAudio as MediaType,
                BusDirections_::kInput as BusDirection,
                1,
                &mut sidechain_info,
            )
        },
        kResultOk
    );
    assert_eq!(sidechain_info.channelCount, 2);
    assert_eq!(sidechain_info.busType, BusTypes_::kAux as BusType);
    assert_eq!(sidechain_info.flags, 0);

    let mut arrangement = SpeakerArrangement::default();
    assert_eq!(
        unsafe {
            processor.getBusArrangement(BusDirections_::kInput as BusDirection, 1, &mut arrangement)
        },
        kResultOk
    );
    assert_eq!(arrangement, SpeakerArr::kStereo);
}

#[test]
fn processor_keeps_main_audio_when_optional_sidechain_bus_is_inactive() {
    let shared = Arc::new(PumpVst3Shared::new());
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut fixture = stereo_process_fixture(32, 9.0);
    fixture
        ._input_buses
        .push(unsafe { mem::zeroed::<AudioBusBuffers>() });
    fixture.process_data.numInputs = 2;
    fixture.process_data.inputs = fixture._input_buses.as_mut_ptr();

    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert!(!shared.status.sidechain_available());
    assert!(fixture.output_left.iter().any(|sample| *sample != 9.0));
    assert!(fixture.output_right.iter().any(|sample| *sample != 9.0));
}

#[test]
fn processor_honors_optional_sidechain_bus_activation_state() {
    let shared = Arc::new(PumpVst3Shared::new());
    shared
        .params
        .set_trigger_mode(crate::params::TRIGGER_MODE_SIDECHAIN as f32);
    let processor = PumpVst3Processor::new(Arc::clone(&shared));
    let mut fixture = stereo_process_fixture(32, 9.0);
    let mut sidechain_left = vec![0.0; 32];
    let mut sidechain_right = vec![0.0; 32];
    sidechain_right[0] = 0.5;
    let mut sidechain_channel_buffers =
        vec![sidechain_left.as_mut_ptr(), sidechain_right.as_mut_ptr()];
    fixture._input_buses.push(AudioBusBuffers {
        numChannels: 2,
        silenceFlags: 0,
        __field0: AudioBusBuffers__type0 {
            channelBuffers32: sidechain_channel_buffers.as_mut_ptr(),
        },
    });
    fixture.process_data.numInputs = 2;
    fixture.process_data.inputs = fixture._input_buses.as_mut_ptr();

    assert_eq!(
        unsafe {
            processor.activateBus(
                MediaTypes_::kAudio as MediaType,
                BusDirections_::kInput as BusDirection,
                0,
                1,
            )
        },
        kResultOk
    );
    assert_eq!(
        unsafe {
            processor.activateBus(
                MediaTypes_::kAudio as MediaType,
                BusDirections_::kOutput as BusDirection,
                0,
                1,
            )
        },
        kResultOk
    );
    assert_eq!(
        unsafe {
            processor.activateBus(
                MediaTypes_::kAudio as MediaType,
                BusDirections_::kInput as BusDirection,
                1,
                1,
            )
        },
        kResultOk
    );
    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert!(shared.status.sidechain_available());

    assert_eq!(
        unsafe {
            processor.activateBus(
                MediaTypes_::kAudio as MediaType,
                BusDirections_::kInput as BusDirection,
                1,
                0,
            )
        },
        kResultOk
    );
    assert_eq!(
        unsafe { processor.process(&mut fixture.process_data) },
        process_ok()
    );
    assert!(!shared.status.sidechain_available());
}

#[test]
#[cfg(target_os = "macos")]
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
    assert_eq!(rect.bottom - rect.top, 900);
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
fn processor_publishes_input_waveform_by_default_and_clears_it_when_input_disappears() {
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
        .expect("VST3 input should publish a waveform snapshot");
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
