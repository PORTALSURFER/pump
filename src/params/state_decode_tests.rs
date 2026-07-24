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

fn payload_for_state_version(params: &PumpParams, version: u32) -> Vec<u8> {
    let mut payload = encode_state_payload(params);
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
        5..=7 => {}
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
fn decode_v7_state_migrates_trigger_mode_to_host() {
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
fn decode_rejects_trailing_bytes_without_mutating_state() {
    let params = sample_params();
    let mut payload = encode_state_payload(&params);
    payload.push(0xAA);

    assert_decode_error_preserves_state(&params, &payload, "unexpected trailing state bytes");
}
