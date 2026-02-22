use super::{
    clamp_sync_division, decode_state_payload, encode_state_payload, sync_division_index_from_text,
    PumpParams, PumpPreset, PumpPresetBank, SavePresetOutcome, MAX_PRESET_NAME_CHARS,
    MAX_SYNC_DIVISION,
};
use crate::curve::{CurveNode, CurveSegment, EditableCurve, CURVE_TABLE_LEN};

#[test]
fn sync_division_parser_accepts_labels() {
    assert_eq!(sync_division_index_from_text("1/4"), Some(4));
    assert_eq!(sync_division_index_from_text("2 bars"), Some(7));
    assert_eq!(sync_division_index_from_text("bogus"), None);
}

#[test]
fn sync_division_clamping_is_bounded() {
    assert_eq!(clamp_sync_division(-2.0), 0);
    assert_eq!(clamp_sync_division(999.0), MAX_SYNC_DIVISION as usize);
}

#[test]
fn state_roundtrip_preserves_values() {
    let params = PumpParams::new();
    params.set_mix(0.23);
    params.set_depth(0.81);
    params.set_phase_offset(0.42);
    params.set_output_gain_db(-3.0);
    params.set_sync_division(6.0);
    params.set_editable_curve(&EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 1.0 },
            CurveNode { x: 0.2, y: 0.1 },
            CurveNode { x: 1.0, y: 0.9 },
        ],
        segments: vec![
            CurveSegment { tension: -0.5 },
            CurveSegment { tension: 0.4 },
        ],
    });

    let payload = encode_state_payload(&params);

    let restored = PumpParams::new();
    decode_state_payload(&restored, &payload).expect("state should decode");

    assert!((restored.mix() - 0.23).abs() < 1.0e-6);
    assert!((restored.depth() - 0.81).abs() < 1.0e-6);
    assert!((restored.phase_offset() - 0.42).abs() < 1.0e-6);
    assert!((restored.output_gain_db() + 3.0).abs() < 1.0e-6);
    assert_eq!(restored.sync_division(), 6);
    let editable = restored.editable_curve_snapshot();
    assert_eq!(editable.nodes.len(), 3);
    assert_eq!(editable.segments.len(), 2);
}

#[test]
fn legacy_payload_still_decodes() {
    let mut legacy = Vec::with_capacity(4 * (5 + CURVE_TABLE_LEN));
    legacy.extend_from_slice(&0.4_f32.to_le_bytes());
    legacy.extend_from_slice(&0.6_f32.to_le_bytes());
    legacy.extend_from_slice(&0.2_f32.to_le_bytes());
    legacy.extend_from_slice(&(-1.5_f32).to_le_bytes());
    legacy.extend_from_slice(&4.0_f32.to_le_bytes());
    for index in 0..CURVE_TABLE_LEN {
        let phase = index as f32 / (CURVE_TABLE_LEN - 1) as f32;
        legacy.extend_from_slice(&phase.to_le_bytes());
    }

    let restored = PumpParams::new();
    decode_state_payload(&restored, &legacy).expect("legacy state should decode");
    assert!((restored.mix() - 0.4).abs() < 1.0e-6);
    assert!((restored.depth() - 0.6).abs() < 1.0e-6);
    assert_eq!(restored.sync_division(), 4);
    let editable = restored.editable_curve_snapshot();
    assert!(editable.nodes.len() >= 2);
    assert_eq!(editable.segments.len(), editable.nodes.len() - 1);
}

#[test]
fn default_curve_is_simple_after_reset() {
    let params = PumpParams::new();
    let curve = params.editable_curve_snapshot();
    assert_eq!(curve.nodes.len(), 4);
    assert_eq!(curve.segments.len(), 3);

    params.set_editable_curve(&EditableCurve {
        nodes: vec![CurveNode { x: 0.0, y: 1.0 }, CurveNode { x: 1.0, y: 0.0 }],
        segments: vec![CurveSegment { tension: 0.0 }],
    });
    params.reset_curve_to_default();
    let reset_curve = params.editable_curve_snapshot();
    assert_eq!(reset_curve.nodes.len(), 4);
    assert_eq!(reset_curve.segments.len(), 3);
}

#[test]
fn sync_division_change_does_not_mutate_curve_or_revision() {
    let params = PumpParams::new();
    let custom_curve = EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 1.0 },
            CurveNode { x: 0.3, y: 0.2 },
            CurveNode { x: 1.0, y: 0.8 },
        ],
        segments: vec![
            CurveSegment { tension: -0.3 },
            CurveSegment { tension: 0.2 },
        ],
    }
    .normalized();
    params.set_editable_curve(&custom_curve);
    let curve_before = params.editable_curve_snapshot();
    let revision_before = params.curve_revision();

    params.set_sync_division(2.0);

    assert_eq!(params.sync_division(), 2);
    assert_eq!(
        params.editable_curve_snapshot(),
        curve_before,
        "sync division changes must preserve editable curve"
    );
    assert_eq!(
        params.curve_revision(),
        revision_before,
        "sync division changes must not bump curve revision"
    );
}

#[test]
fn preset_add_rename_and_load_roundtrip_current_state() {
    let params = PumpParams::new();
    params.set_mix(0.31);
    params.set_depth(0.91);
    params.set_phase_offset(0.27);
    params.set_output_gain_db(-6.0);
    params.set_sync_division(6.0);
    params.set_editable_curve(&EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 0.9 },
            CurveNode { x: 0.4, y: 0.2 },
            CurveNode { x: 1.0, y: 0.9 },
        ],
        segments: vec![
            CurveSegment { tension: -0.2 },
            CurveSegment { tension: 0.3 },
        ],
    });

    let inserted = params
        .add_preset_from_current_state()
        .expect("preset insertion should succeed");
    assert_eq!(inserted, 1);
    assert!(params.rename_preset(inserted, "My Wide Preset Name That Should Clamp"));

    let bank = params.preset_bank_snapshot();
    assert_eq!(bank.selected, 1);
    assert_eq!(bank.presets.len(), 2);
    assert!(bank.presets[1].name.chars().count() <= MAX_PRESET_NAME_CHARS);

    params.set_mix(0.05);
    params.set_depth(0.1);
    params.set_sync_division(1.0);
    params.load_preset(1).expect("preset load should succeed");
    assert!((params.mix() - bank.presets[1].mix).abs() < 1.0e-6);
    assert!((params.depth() - bank.presets[1].depth).abs() < 1.0e-6);
    assert_eq!(params.sync_division(), bank.presets[1].sync_division);
}

#[test]
fn preset_bank_roundtrips_through_state_payload() {
    let params = PumpParams::new();
    params
        .add_preset_from_current_state()
        .expect("preset insertion should succeed");
    assert!(params.rename_preset(1, "Verse"));
    params.load_preset(1).expect("preset load should succeed");
    let payload = encode_state_payload(&params);

    let restored = PumpParams::new();
    decode_state_payload(&restored, &payload).expect("state should decode");
    let bank = restored.preset_bank_snapshot();
    assert_eq!(bank.presets.len(), 2);
    assert_eq!(bank.selected, 1);
    assert_eq!(bank.presets[1].name, "Verse");
    assert!(!bank.presets[0].is_read_only);
}

#[test]
fn set_preset_bank_preserves_user_presets_without_inserting_init() {
    let params = PumpParams::new();
    params.set_preset_bank(PumpPresetBank {
        selected: 1,
        presets: vec![
            PumpPreset {
                name: "Live A".to_string(),
                is_read_only: false,
                mix: 0.11,
                depth: 0.22,
                phase_offset: 0.33,
                output_gain_db: -1.0,
                sync_division: 2,
                editable_curve: params.editable_curve_snapshot(),
            },
            PumpPreset {
                name: "Live B".to_string(),
                is_read_only: false,
                mix: 0.77,
                depth: 0.66,
                phase_offset: 0.55,
                output_gain_db: -2.0,
                sync_division: 4,
                editable_curve: params.editable_curve_snapshot(),
            },
        ],
    });

    let bank = params.preset_bank_snapshot();
    assert_eq!(bank.presets.len(), 2);
    assert_eq!(bank.selected, 1);
    assert_eq!(bank.presets[0].name, "Live A");
    assert!((bank.presets[0].mix - 0.11).abs() < 1.0e-6);
    assert_eq!(bank.presets[1].name, "Live B");
    assert!((bank.presets[1].mix - 0.77).abs() < 1.0e-6);
}

#[test]
fn save_by_name_overwrites_case_insensitive_match() {
    let params = PumpParams::new();
    params
        .add_preset_from_current_state()
        .expect("preset insertion should succeed");
    assert!(params.rename_preset(1, "Verse"));
    params.set_mix(0.12);
    params.set_depth(0.33);

    let outcome = params.save_current_state_by_name(" verse ");
    assert_eq!(outcome, SavePresetOutcome::Overwritten { index: 1 });
    let bank = params.preset_bank_snapshot();
    assert_eq!(bank.selected, 1);
    assert_eq!(bank.presets[1].name, "Verse");
    assert!((bank.presets[1].mix - 0.12).abs() < 1.0e-6);
    assert!((bank.presets[1].depth - 0.33).abs() < 1.0e-6);
}

#[test]
fn save_by_name_creates_when_no_match_exists() {
    let params = PumpParams::new();
    params.set_mix(0.77);
    let outcome = params.save_current_state_by_name("Hook");
    assert_eq!(outcome, SavePresetOutcome::Created { index: 1 });
    let bank = params.preset_bank_snapshot();
    assert_eq!(bank.presets.len(), 2);
    assert_eq!(bank.selected, 1);
    assert_eq!(bank.presets[1].name, "Hook");
    assert!((bank.presets[1].mix - 0.77).abs() < 1.0e-6);
}

#[test]
fn init_preset_is_writable_for_rename_and_save() {
    let params = PumpParams::new();
    params.set_mix(0.61);
    assert!(params.rename_preset(0, "Init2"));
    assert_eq!(
        params.save_current_state_by_name("Init2"),
        SavePresetOutcome::Overwritten { index: 0 }
    );
    let bank = params.preset_bank_snapshot();
    assert_eq!(bank.presets[0].name, "Init2");
    assert!((bank.presets[0].mix - 0.61).abs() < 1.0e-6);
    assert!(!bank.presets[0].is_read_only);
}

#[test]
fn save_by_name_blocks_when_preset_bank_is_full() {
    let params = PumpParams::new();
    for _ in 1..16 {
        params
            .add_preset_from_current_state()
            .expect("preset insertion should succeed");
    }
    assert_eq!(params.preset_bank_snapshot().presets.len(), 16);
    assert_eq!(
        params.save_current_state_by_name("BrandNew"),
        SavePresetOutcome::BlockedFull
    );
}
