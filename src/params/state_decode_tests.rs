use super::{decode_state_payload, encode_state_payload, PumpParams};

const OFFSET_VERSION: usize = 4;
const OFFSET_NODE_COUNT: usize = 28;
const CURVE_HEADER_BYTES: usize = 32;

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
    let node_count = read_u32(payload, OFFSET_NODE_COUNT) as usize;
    let curve_bytes = node_count * 8 + node_count.saturating_sub(1) * 4;
    CURVE_HEADER_BYTES + curve_bytes + 8
}

fn first_preset_node_count_offset(payload: &[u8]) -> usize {
    let version = read_u32(payload, OFFSET_VERSION);
    assert!(
        version >= 4,
        "test assumes payload format includes preset read-only flag"
    );
    let preset_start = first_preset_start(payload);
    let name_len = read_u32(payload, preset_start) as usize;
    preset_start + 4 + name_len + (5 * 4) + 1
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
fn decode_rejects_trailing_bytes_without_mutating_state() {
    let params = sample_params();
    let mut payload = encode_state_payload(&params);
    payload.push(0xAA);

    assert_decode_error_preserves_state(&params, &payload, "unexpected trailing state bytes");
}
