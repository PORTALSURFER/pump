use super::*;
use std::mem;

#[test]
fn controller_reports_expected_parameter_count() {
    let controller = PumpVst3Controller::new(Arc::new(PumpVst3Shared::new()));
    let count = unsafe { controller.getParameterCount() };
    assert_eq!(count, 4);
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
