use super::*;
use std::mem;
use std::ptr;

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
    assert_eq!(count, 4);
}

#[test]
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
fn view_enforces_minimum_size() {
    let (preferred_width, preferred_height) = preferred_window_size();
    let view = HostedVst3View::new(
        PumpVst3GuiAdapter::new(Arc::new(PumpVst3Shared::new())),
        preferred_width,
        preferred_height,
    );
    let mut rect = view_rect(10, 10);
    let result = unsafe { view.checkSizeConstraint(&mut rect) };
    assert_eq!(result, kResultOk);
    assert!(rect.right - rect.left >= preferred_width as i32);
    assert!(rect.bottom - rect.top >= preferred_height as i32);
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
