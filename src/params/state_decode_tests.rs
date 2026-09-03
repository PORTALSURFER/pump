use super::{decode_state_payload, encode_state_payload, PumpParams};

const OFFSET_VERSION: usize = 4;
const OFFSET_NODE_COUNT: usize = 32;

#[derive(Debug, PartialEq, Eq)]
struct StateFingerprint {
    payload: Vec<u8>,
    curve_revision: u32,
}

fn snapshot_state(params: &PumpParams) -> StateFingerprint {
    StateFingerprint {
        payload: encode_state_payload(params),
        curve_revision: params.curve_revision(),
    }
}

fn assert_decode_error_preserves_state(
    params: &PumpParams,
    payload: &[u8],
    expected_error: &'static str,
) {
    let before = snapshot_state(params);
    let result = decode_state_payload(params, payload);
    assert_eq!(result, Err(expected_error));
    let after = snapshot_state(params);
    assert_eq!(after, before, "decode failure must not mutate active state");
}

fn read_u32(payload: &[u8], offset: usize) -> u32 {
    let bytes = payload
        .get(offset..offset + 4)
        .expect("u32 offset should be in bounds");
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn write_u32(payload: &mut [u8], offset: usize, value: u32) {
    let target = payload
        .get_mut(offset..offset + 4)
        .expect("u32 offset should be in bounds");
    target.copy_from_slice(&value.to_le_bytes());
}

fn write_f32(payload: &mut [u8], offset: usize, value: f32) {
    payload[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn first_preset_start(payload: &[u8]) -> usize {
    let (node_offset, curve_header_bytes) = if (2..=65).contains(&read_u32(payload, 32)) {
        (32, 36)
    } else {
        (28, 32)
    };
    let node_count = read_u32(payload, node_offset) as usize;
    let curve_bytes = node_count * 8 + node_count.saturating_sub(1) * 4;
    curve_header_bytes + curve_bytes + 8
}

fn first_preset_node_count_offset(payload: &[u8]) -> usize {
    let preset_start = first_preset_start(payload);
    let name_len = read_u32(payload, preset_start) as usize;
    let fields = if read_u32(payload, OFFSET_VERSION) >= 7 {
        6
    } else {
        5
    };
    preset_start + 4 + name_len + (fields * 4) + 1
}

fn first_preset_quick_slot_count_offset(payload: &[u8]) -> usize {
    let node_count_offset = first_preset_node_count_offset(payload);
    let node_count = read_u32(payload, node_count_offset) as usize;
    let curve_bytes = node_count * 8 + node_count.saturating_sub(1) * 4;
    node_count_offset + 4 + curve_bytes
}

fn skip_encoded_curve(payload: &[u8], mut offset: usize, with_phase_metadata: bool) -> usize {
    let node_count = read_u32(payload, offset) as usize;
    offset += 4 + node_count * 8 + node_count.saturating_sub(1) * 4;
    if with_phase_metadata
        && payload
            .get(offset..offset.saturating_add(4))
            .is_some_and(|bytes| bytes.len() == 4)
        && read_u32(payload, offset) == u32::from_le_bytes(*b"PHAS")
    {
        offset += 8;
        let source_node_count = read_u32(payload, offset) as usize;
        offset += 4 + source_node_count * 8 + source_node_count.saturating_sub(1) * 4;
    }
    offset
}

fn preset_swing_offsets(payload: &[u8]) -> Vec<usize> {
    let mut offset = first_preset_start(payload);
    let preset_count = read_u32(payload, offset - 4) as usize;
    let mut swing_offsets = Vec::with_capacity(preset_count);

    for _ in 0..preset_count {
        let name_len = read_u32(payload, offset) as usize;
        offset += 4 + name_len + 6 * 4 + 1;
        offset = skip_encoded_curve(payload, offset, true);

        let quick_slot_count = read_u32(payload, offset) as usize;
        offset += 4;
        for _ in 0..quick_slot_count {
            offset = skip_encoded_curve(payload, offset, true);
        }

        // Trigger, smooth, processing mode, favorite, then Swing.
        offset += 4 + 4 + 4 + 1;
        swing_offsets.push(offset);
        offset += 4;
    }

    swing_offsets
}

fn v14_extension_start(payload: &[u8]) -> usize {
    let mut offset = first_preset_start(payload);
    let preset_count = read_u32(payload, offset - 4) as usize;
    for _ in 0..preset_count {
        let name_len = read_u32(payload, offset) as usize;
        offset += 4 + name_len + 6 * 4 + 1;
        offset = skip_encoded_curve(payload, offset, true);
        let quick_slot_count = read_u32(payload, offset) as usize;
        offset += 4;
        for _ in 0..quick_slot_count {
            offset = skip_encoded_curve(payload, offset, true);
        }
        offset += 4 + 4 + 4 + 1 + 4;
    }
    offset + 20
}

fn v15_extension_start(payload: &[u8]) -> usize {
    let mut offset = first_preset_start(payload);
    let preset_count = read_u32(payload, offset - 4) as usize;
    for _ in 0..preset_count {
        let name_len = read_u32(payload, offset) as usize;
        offset += 4 + name_len + 6 * 4 + 1;
        offset = skip_encoded_curve(payload, offset, true);
        let quick_slot_count = read_u32(payload, offset) as usize;
        offset += 4;
        for _ in 0..quick_slot_count {
            offset = skip_encoded_curve(payload, offset, true);
        }
        offset += 4 + 4 + 4 + 1 + 4 + 8;
    }
    offset + 20
}

fn v17_extension_start(payload: &[u8]) -> usize {
    let mut offset = first_preset_start(payload);
    let preset_count = read_u32(payload, offset - 4) as usize;
    for _ in 0..preset_count {
        let name_len = read_u32(payload, offset) as usize;
        offset += 4 + name_len + 6 * 4 + 1;
        offset = skip_encoded_curve(payload, offset, true);
        let quick_slot_count = read_u32(payload, offset) as usize;
        offset += 4;
        for _ in 0..quick_slot_count {
            offset = skip_encoded_curve(payload, offset, true);
        }
        offset += 4 + 4 + 4 + 1 + 4 + 8 + 4;
    }
    offset + 20
}

fn preset_delay_offsets(payload: &[u8]) -> Vec<usize> {
    let mut offset = first_preset_start(payload);
    let preset_count = read_u32(payload, offset - 4) as usize;
    let mut delay_offsets = Vec::with_capacity(preset_count);
    for _ in 0..preset_count {
        let name_len = read_u32(payload, offset) as usize;
        offset += 4 + name_len + 6 * 4 + 1;
        offset = skip_encoded_curve(payload, offset, true);
        let quick_slot_count = read_u32(payload, offset) as usize;
        offset += 4;
        for _ in 0..quick_slot_count {
            offset = skip_encoded_curve(payload, offset, true);
        }
        offset += 4 + 4 + 4 + 1 + 4 + 8;
        delay_offsets.push(offset);
        offset += 4;
    }
    delay_offsets
}

fn remove_v17_delay_fields(payload: &mut Vec<u8>) {
    let mut delay_offsets = preset_delay_offsets(payload);
    let extension_start = v17_extension_start(payload);
    let (_, _, a_end) = sound_state_offsets(payload, extension_start + 4);
    let b_start = a_end + 12;
    let (_, _, b_end) = sound_state_offsets(payload, b_start);
    let stored_a_start = b_end + 12;
    let (_, _, stored_a_end) = sound_state_offsets(payload, stored_a_start);
    let stored_b_start = stored_a_end + 12;
    let (_, _, stored_b_end) = sound_state_offsets(payload, stored_b_start);

    delay_offsets.extend([
        a_end + 8,
        b_end + 8,
        stored_a_end + 8,
        stored_b_end + 8,
        stored_b_end + 20,
    ]);
    delay_offsets.sort_unstable();
    for offset in delay_offsets.into_iter().rev() {
        payload.drain(offset..offset + 4);
    }
}

fn preset_timing_offsets(payload: &[u8]) -> Vec<usize> {
    let mut offset = first_preset_start(payload);
    let preset_count = read_u32(payload, offset - 4) as usize;
    let mut timing_offsets = Vec::with_capacity(preset_count);
    for _ in 0..preset_count {
        let name_len = read_u32(payload, offset) as usize;
        offset += 4 + name_len + 6 * 4 + 1;
        offset = skip_encoded_curve(payload, offset, true);
        let quick_slot_count = read_u32(payload, offset) as usize;
        offset += 4;
        for _ in 0..quick_slot_count {
            offset = skip_encoded_curve(payload, offset, true);
        }
        offset += 4 + 4 + 4 + 1 + 4;
        timing_offsets.push(offset);
        offset += 8;
    }
    timing_offsets
}

fn first_preset_trigger_and_mode_offsets(payload: &[u8]) -> (usize, usize) {
    let mut offset = first_preset_start(payload);
    let name_len = read_u32(payload, offset) as usize;
    offset += 4 + name_len + 6 * 4 + 1;
    offset = skip_encoded_curve(payload, offset, true);
    let quick_slot_count = read_u32(payload, offset) as usize;
    offset += 4;
    for _ in 0..quick_slot_count {
        offset = skip_encoded_curve(payload, offset, true);
    }
    (offset, offset + 8)
}

fn sound_state_offsets(payload: &[u8], start: usize) -> (usize, usize, usize) {
    let trigger = start + 24;
    let mode = start + 32;
    let mut offset = start + 40;
    offset = skip_encoded_curve(payload, offset, true);
    let quick_slot_count = read_u32(payload, offset) as usize;
    offset += 4;
    for _ in 0..quick_slot_count {
        offset = skip_encoded_curve(payload, offset, true);
    }
    (trigger, mode, offset)
}

pub(crate) fn payload_for_state_version(params: &PumpParams, version: u32) -> Vec<u8> {
    let mut payload = encode_state_payload(params);
    if version < 17 {
        // Delay was appended to every current preset/sound record and to the
        // top-level timing extension in v17. Remove those fields before
        // applying the older v16/v15 migrations below.
        remove_v17_delay_fields(&mut payload);
    }
    if version < 16 {
        // Stored/reference snapshots were appended in v16. Remove both
        // variable-length records before applying the older field removals.
        let extension_start = v15_extension_start(&payload);
        let (_, _, a_end) = sound_state_offsets(&payload, extension_start + 4);
        let b_start = a_end + 8;
        let (_, _, b_end) = sound_state_offsets(&payload, b_start);
        let stored_a_start = b_end + 8;
        let (_, _, stored_a_end) = sound_state_offsets(&payload, stored_a_start);
        let stored_b_start = stored_a_end + 8;
        let (_, _, stored_b_end) = sound_state_offsets(&payload, stored_b_start);
        payload.drain(stored_a_start..stored_b_end + 8);
    }
    if version < 15 {
        let preset_timing_offsets = preset_timing_offsets(&payload);
        let extension_start = v15_extension_start(&payload);
        let (_, _, a_end) = sound_state_offsets(&payload, extension_start + 4);
        let b_start = a_end + 8;
        let (_, _, b_end) = sound_state_offsets(&payload, b_start);
        payload.drain(b_end..b_end + 8);
        payload.drain(a_end..a_end + 8);
        payload.truncate(payload.len().saturating_sub(8));
        for timing_offset in preset_timing_offsets.into_iter().rev() {
            payload.drain(timing_offset..timing_offset + 8);
        }
    }
    if version < 14 {
        payload.truncate(v14_extension_start(&payload));
    }
    if version < 13 {
        // Swing was added to the top-level active state and each preset in v13.
        let preset_swing_offsets = preset_swing_offsets(&payload);
        // The trailing field is the active-state Swing; remove it separately
        // from the variable-length preset records.
        payload.truncate(payload.len().saturating_sub(4));
        for swing_offset in preset_swing_offsets.into_iter().rev() {
            payload.drain(swing_offset..swing_offset + 4);
        }
    }
    if version < 12 {
        // Host bypass was added only to the top-level active state in v12.
        payload.truncate(payload.len().saturating_sub(4));
    }
    if version < 11 {
        // Processing mode was added in v11, once in the active record and
        // once per preset.
        payload.truncate(payload.len().saturating_sub(4));
        let mode_start = payload.len().saturating_sub(13);
        payload.drain(mode_start..mode_start + 4);
    }
    if version < 10 {
        // Evaluated gain smoothing was added in v10, once in the active
        // record and once per preset.
        payload.truncate(payload.len().saturating_sub(4));
        let smooth_start = payload.len().saturating_sub(9);
        payload.drain(smooth_start..smooth_start + 4);
    }
    if version < 9 {
        // Favorite metadata was added in v9, once per preset.
        payload.remove(payload.len().saturating_sub(5));
    }
    if version < 8 {
        // Trigger mode was added in v8, once per preset and once for the active
        // top-level state. Remove those fields before emulating an old payload.
        payload.truncate(payload.len().saturating_sub(8));
    }
    if version < 7 {
        // Remove the v7 Floor field from the top-level record and first preset
        // so the migration test represents a real pre-v7 payload.
        payload.drain(16..20);
        let preset_start = first_preset_start(&payload);
        let name_len = read_u32(&payload, preset_start) as usize;
        let floor_offset = preset_start + 4 + name_len + 8;
        payload.drain(floor_offset..floor_offset + 4);
    }
    write_u32(&mut payload, OFFSET_VERSION, version);
    let preset_bank_offset = first_preset_start(&payload) - 8;
    match version {
        2 => payload.truncate(preset_bank_offset),
        3 => {
            let flag_offset = first_preset_node_count_offset(&payload) - 1;
            let quick_slot_offset = first_preset_quick_slot_count_offset(&payload);
            payload.remove(flag_offset);
            payload.truncate(quick_slot_offset - 1);
        }
        4 => {
            let quick_slot_offset = first_preset_quick_slot_count_offset(&payload);
            payload.truncate(quick_slot_offset);
        }
        5..=17 => {}
        _ => panic!("unsupported test state version"),
    }
    payload
}

fn sample_params() -> PumpParams {
    let params = PumpParams::new();
    params.set_mix(0.73);
    params.set_depth(0.19);
    params.set_phase_offset(0.42);
    params.set_output_gain_db(-3.5);
    params.set_sync_division(6.0);
    params
}

#[test]
fn decode_v7_state_defaults_trigger_mode_to_host() {
    let params = sample_params();
    params.set_trigger_mode(super::TRIGGER_MODE_SIDECHAIN as f32);
    let payload = payload_for_state_version(&params, 7);

    let restored = PumpParams::new();
    decode_state_payload(&restored, &payload).expect("v7 state should decode");
    assert_eq!(restored.trigger_mode(), super::DEFAULT_TRIGGER_MODE);
    assert_eq!(
        restored.preset_bank_snapshot().presets[0].trigger_mode,
        super::DEFAULT_TRIGGER_MODE
    );
}

#[test]
fn current_state_maps_legacy_sidechain_and_punch_values_to_supported_modes() {
    let source = sample_params();
    let mut payload = encode_state_payload(&source);
    let extension_start = v17_extension_start(&payload);
    write_f32(
        &mut payload,
        extension_start - 20,
        super::TRIGGER_MODE_SIDECHAIN as f32,
    );
    write_f32(
        &mut payload,
        extension_start - 12,
        super::PROCESSING_MODE_PUNCH as f32,
    );
    let (preset_trigger, preset_mode) = first_preset_trigger_and_mode_offsets(&payload);
    write_u32(
        &mut payload,
        preset_trigger,
        super::TRIGGER_MODE_SIDECHAIN as u32,
    );
    write_u32(
        &mut payload,
        preset_mode,
        super::PROCESSING_MODE_PUNCH as u32,
    );
    let (a_trigger, a_mode, a_end) = sound_state_offsets(&payload, extension_start + 4);
    let b_start = a_end + 12;
    let (b_trigger, b_mode, b_end) = sound_state_offsets(&payload, b_start);
    assert_eq!(a_end + 12, b_start);
    let (stored_a_trigger, stored_a_mode, stored_a_end) = sound_state_offsets(&payload, b_end + 12);
    let (stored_b_trigger, stored_b_mode, stored_b_end) =
        sound_state_offsets(&payload, stored_a_end + 12);
    assert_eq!(stored_b_end + 12, payload.len() - 12);
    write_u32(
        &mut payload,
        a_trigger,
        super::TRIGGER_MODE_SIDECHAIN as u32,
    );
    write_u32(&mut payload, a_mode, super::PROCESSING_MODE_PUNCH as u32);
    write_u32(
        &mut payload,
        b_trigger,
        super::TRIGGER_MODE_SIDECHAIN as u32,
    );
    write_u32(&mut payload, b_mode, super::PROCESSING_MODE_PUNCH as u32);
    write_u32(
        &mut payload,
        stored_a_trigger,
        super::TRIGGER_MODE_SIDECHAIN as u32,
    );
    write_u32(
        &mut payload,
        stored_a_mode,
        super::PROCESSING_MODE_PUNCH as u32,
    );
    write_u32(
        &mut payload,
        stored_b_trigger,
        super::TRIGGER_MODE_SIDECHAIN as u32,
    );
    write_u32(
        &mut payload,
        stored_b_mode,
        super::PROCESSING_MODE_PUNCH as u32,
    );

    let restored = PumpParams::new();
    decode_state_payload(&restored, &payload).expect("current state should decode");
    assert_eq!(restored.trigger_mode(), super::TRIGGER_MODE_HOST);
    assert_eq!(restored.mode(), super::PROCESSING_MODE_CLASSIC);
    let preset = &restored.preset_bank_snapshot().presets[0];
    assert_eq!(preset.trigger_mode, super::TRIGGER_MODE_HOST);
    assert_eq!(preset.mode, super::PROCESSING_MODE_CLASSIC);
    for side in [super::SoundSide::A, super::SoundSide::B] {
        let state = restored.sound_state_snapshot(side);
        assert_eq!(state.trigger_mode, super::TRIGGER_MODE_HOST);
        assert_eq!(state.mode, super::PROCESSING_MODE_CLASSIC);
    }
}

#[test]
fn decode_v11_state_migrates_bypass_to_active() {
    let params = sample_params();
    params.set_bypass(1.0);
    let payload = payload_for_state_version(&params, 11);

    let restored = PumpParams::new();
    restored.set_bypass(1.0);
    decode_state_payload(&restored, &payload).expect("v11 state should decode");
    assert!(!restored.bypassed());
}

#[test]
fn decode_rejects_malformed_v12_bypass_without_mutating_state() {
    let params = sample_params();
    params.set_bypass(1.0);
    let mut payload = payload_for_state_version(&params, 12);
    payload.truncate(payload.len().saturating_sub(2));
    assert_decode_error_preserves_state(&params, &payload, "invalid bypass field");
}

#[test]
fn decode_v12_preserves_controls_and_defaults_swing() {
    let source = sample_params();
    source.set_trigger_mode(super::TRIGGER_MODE_SIDECHAIN as f32);
    source.set_smooth(0.67);
    source.set_mode(super::PROCESSING_MODE_PUNCH as f32);
    source.set_bypass(1.0);
    source.set_swing(0.42);

    let mut bank = source.preset_bank_snapshot();
    for preset in &mut bank.presets {
        preset.trigger_mode = super::TRIGGER_MODE_SIDECHAIN;
        preset.smooth = 0.67;
        preset.mode = super::PROCESSING_MODE_PUNCH;
        preset.swing = 0.42;
    }
    source.set_preset_bank_without_persistence(bank);

    let payload = payload_for_state_version(&source, 12);
    let restored = PumpParams::new();
    decode_state_payload(&restored, &payload).expect("v12 state should decode");

    assert_eq!(restored.trigger_mode(), super::TRIGGER_MODE_HOST);
    assert!((restored.smooth() - 0.67).abs() < 1.0e-6);
    assert_eq!(restored.mode(), super::PROCESSING_MODE_CLASSIC);
    assert!(restored.bypassed());
    assert_eq!(restored.swing(), super::DEFAULT_SWING);
    assert_eq!(restored.timing_mode(), super::DEFAULT_TIMING_MODE);
    assert_eq!(restored.free_rate_hz(), super::DEFAULT_FREE_RATE_HZ);

    let bank = restored.preset_bank_snapshot();
    assert!(!bank.presets.is_empty());
    for preset in bank.presets {
        assert_eq!(preset.trigger_mode, super::TRIGGER_MODE_HOST);
        assert!((preset.smooth - 0.67).abs() < 1.0e-6);
        assert_eq!(preset.mode, super::PROCESSING_MODE_CLASSIC);
        assert_eq!(preset.swing, super::DEFAULT_SWING);
        assert_eq!(preset.timing_mode, super::DEFAULT_TIMING_MODE);
        assert_eq!(preset.free_rate_hz, super::DEFAULT_FREE_RATE_HZ);
    }
}

fn assert_legacy_ab_state_seeds_stored_references(version: u32) {
    let source = sample_params();
    source.set_mix(0.23);
    assert!(source.copy_active_to_inactive());
    assert!(source.set_active_sound(super::SoundSide::B));
    source.set_mix(0.71);

    let payload = payload_for_state_version(&source, version);
    let restored = PumpParams::new();
    decode_state_payload(&restored, &payload)
        .unwrap_or_else(|error| panic!("state v{version} should decode: {error}"));

    for side in [super::SoundSide::A, super::SoundSide::B] {
        assert_eq!(
            restored.sound_state_snapshot(side),
            restored.stored_sound_state_snapshot(side),
            "state v{version} should seed stored sound {} from working state",
            side.label()
        );
        assert!(
            !restored.sound_state_is_dirty(side),
            "state v{version} should reopen sound {} clean",
            side.label()
        );
    }
}

#[test]
fn decode_v14_state_seeds_stored_references_from_working_state() {
    assert_legacy_ab_state_seeds_stored_references(14);
}

#[test]
fn decode_v15_state_seeds_stored_references_from_working_state() {
    assert_legacy_ab_state_seeds_stored_references(15);
}

#[test]
fn decode_rejects_invalid_top_level_node_count_without_mutating_state() {
    let params = sample_params();
    let mut payload = encode_state_payload(&params);
    write_u32(&mut payload, OFFSET_NODE_COUNT, 1);

    assert_decode_error_preserves_state(&params, &payload, "invalid node count bounds");
}

#[test]
fn decode_rejects_invalid_preset_count_without_mutating_state() {
    let params = sample_params();
    let mut payload = encode_state_payload(&params);
    let preset_count_offset = first_preset_start(&payload) - 4;
    write_u32(&mut payload, preset_count_offset, 0);

    assert_decode_error_preserves_state(&params, &payload, "invalid preset count bounds");
}

#[test]
fn decode_rejects_invalid_preset_name_utf8_without_mutating_state() {
    let params = sample_params();
    let mut payload = encode_state_payload(&params);
    let name_offset = first_preset_start(&payload) + 4;
    payload[name_offset] = 0xFF;

    assert_decode_error_preserves_state(&params, &payload, "invalid preset name utf8");
}

#[test]
fn decode_rejects_missing_preset_read_only_flag_without_mutating_state() {
    let params = sample_params();
    let mut payload = encode_state_payload(&params);
    let flag_offset = first_preset_node_count_offset(&payload) - 1;
    payload.truncate(flag_offset);

    assert_decode_error_preserves_state(&params, &payload, "invalid preset read-only flag");
}

#[test]
fn decode_rejects_invalid_preset_curve_node_count_without_mutating_state() {
    let params = sample_params();
    let mut payload = encode_state_payload(&params);
    let preset_node_count_offset = first_preset_node_count_offset(&payload);
    write_u32(&mut payload, preset_node_count_offset, 1);

    assert_decode_error_preserves_state(&params, &payload, "invalid node count bounds");
}

#[test]
fn decode_rejects_non_fixed_quick_slot_counts_without_mutating_state() {
    for count in [u32::MAX, 0, 7, 9] {
        let params = sample_params();
        let mut payload = encode_state_payload(&params);
        let count_offset = first_preset_quick_slot_count_offset(&payload);
        write_u32(&mut payload, count_offset, count);

        assert_decode_error_preserves_state(
            &params,
            &payload,
            "invalid preset quick slot count bounds",
        );
    }
}

#[test]
fn decode_rejects_truncated_quick_slots_without_mutating_state() {
    let params = sample_params();
    let mut payload = encode_state_payload(&params);
    let count_offset = first_preset_quick_slot_count_offset(&payload);
    payload.truncate(count_offset + 4);

    assert_decode_error_preserves_state(
        &params,
        &payload,
        "invalid preset quick slot count byte length",
    );
}

#[test]
fn decode_accepts_fixed_quick_slot_count() {
    let params = sample_params();
    let payload = encode_state_payload(&params);

    decode_state_payload(&params, &payload).expect("fixed quick-slot count should decode");
}

#[test]
fn decode_preserves_v2_through_v5_state_compatibility() {
    for version in 2..=5 {
        let params = sample_params();
        let payload = payload_for_state_version(&params, version);

        decode_state_payload(&params, &payload)
            .unwrap_or_else(|error| panic!("state v{version} should decode: {error}"));
        assert_eq!(params.depth_db(), 120.0);
        assert_eq!(params.floor_db(), -60.0);
    }
}

#[test]
fn every_pre_v12_project_state_migrates_bypass_to_active() {
    for version in 2..=11 {
        let source = sample_params();
        source.set_bypass(1.0);
        let payload = payload_for_state_version(&source, version);
        let restored = PumpParams::new();
        restored.set_bypass(1.0);

        decode_state_payload(&restored, &payload)
            .unwrap_or_else(|error| panic!("state v{version} should decode: {error}"));
        assert!(!restored.bypassed(), "state v{version} must migrate Active");
    }
}

#[test]
fn decode_rejects_trailing_bytes_without_mutating_state() {
    let params = sample_params();
    let mut payload = encode_state_payload(&params);
    payload.push(0xAA);

    assert_decode_error_preserves_state(&params, &payload, "unexpected trailing state bytes");
}

#[test]
fn decode_rejects_nonfinite_v14_ab_scalar_without_mutating_state() {
    let params = sample_params();
    let mut payload = encode_state_payload(&params);
    let offset = v17_extension_start(&payload) + 4;
    write_f32(&mut payload, offset, f32::NAN);
    assert_decode_error_preserves_state(&params, &payload, "invalid A/B scalar field");
}
