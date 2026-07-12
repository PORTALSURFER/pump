use super::{
    clamp_sync_division, decode_state_payload, encode_state_payload, seeded_quick_shape_slots,
    sync_division_index_from_text, PresetMutationError, PumpParams, PumpPreset, PumpPresetBank,
    SavePresetOutcome, MAX_PRESET_NAME_CHARS, MAX_SYNC_DIVISION,
};
#[cfg(feature = "vst3")]
use super::{
    clap_id_from_vst3_param_id, format_plain_value_text, parse_plain_value_text,
    vst3_param_info_for_index, PARAM_MIX_ID, PARAM_MIX_NUM, PARAM_OUTPUT_GAIN_ID,
    PARAM_OUTPUT_GAIN_NUM, PARAM_PHASE_OFFSET_ID, PARAM_PHASE_OFFSET_NUM, PARAM_SYNC_DIVISION_ID,
    PARAM_SYNC_DIVISION_NUM,
};
use crate::curve::{CurveNode, CurveSegment, EditableCurve, CURVE_TABLE_LEN};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
fn temp_preset_store_path(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "pump-runtime-preset-store-{label}-{}-{stamp}.bin",
        std::process::id()
    ))
}

fn test_quick_slot_curve(offset: f32) -> EditableCurve {
    EditableCurve {
        nodes: vec![
            CurveNode { x: 0.0, y: 1.0 },
            CurveNode {
                x: (0.12 + offset).clamp(0.05, 0.3),
                y: 0.05,
            },
            CurveNode {
                x: (0.32 + offset).clamp(0.2, 0.6),
                y: 0.62,
            },
            CurveNode { x: 1.0, y: 1.0 },
        ],
        segments: vec![
            CurveSegment { tension: -0.48 },
            CurveSegment { tension: 0.31 },
            CurveSegment { tension: -0.04 },
        ],
    }
    .normalized()
}

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
    assert!((restored.depth() - 1.0).abs() < 1.0e-6);
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
    assert!((restored.depth() - 1.0).abs() < 1.0e-6);
    assert_eq!(restored.sync_division(), 4);
    let editable = restored.editable_curve_snapshot();
    assert!(editable.nodes.len() >= 2);
    assert_eq!(editable.segments.len(), editable.nodes.len() - 1);
    let bank = restored.preset_bank_snapshot();
    assert_eq!(bank.presets[0].quick_slots, seeded_quick_shape_slots());
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
    assert!(params
        .rename_preset(inserted, "My Wide Preset Name That Should Clamp")
        .is_ok());

    let bank = params.preset_bank_snapshot();
    assert_eq!(bank.selected, 1);
    assert_eq!(bank.presets.len(), 2);
    assert!(bank.presets[1].name.chars().count() <= MAX_PRESET_NAME_CHARS);

    params.set_mix(0.05);
    params.set_sync_division(1.0);
    params.load_preset(1).expect("preset load should succeed");
    assert!((params.mix() - bank.presets[1].mix).abs() < 1.0e-6);
    assert_eq!(params.sync_division(), bank.presets[1].sync_division);
}

#[test]
fn preset_bank_roundtrips_through_state_payload() {
    let params = PumpParams::new();
    params
        .add_preset_from_current_state()
        .expect("preset insertion should succeed");
    assert!(params.rename_preset(1, "Verse").is_ok());
    let slot_curve = test_quick_slot_curve(0.08);
    assert!(params.set_selected_quick_slot_curve(3, &slot_curve).is_ok());
    params.load_preset(1).expect("preset load should succeed");
    let payload = encode_state_payload(&params);

    let restored = PumpParams::new();
    decode_state_payload(&restored, &payload).expect("state should decode");
    let bank = restored.preset_bank_snapshot();
    assert_eq!(bank.presets.len(), 2);
    assert_eq!(bank.selected, 1);
    assert_eq!(bank.presets[1].name, "Verse");
    assert!(!bank.presets[0].is_read_only);
    assert_eq!(bank.presets[1].quick_slots[3].curve, slot_curve);
}

#[test]
fn load_preset_persists_selected_index_for_new_instances() {
    let path = temp_preset_store_path("selection-persistence");
    super::preset_store::with_test_persistence_path(path.clone(), || {
        let params = PumpParams::new();
        params
            .add_preset_from_current_state()
            .expect("first preset insertion should succeed");
        params
            .add_preset_from_current_state()
            .expect("second preset insertion should succeed");
        assert_eq!(
            params.preset_bank_snapshot().selected,
            2,
            "second insertion should leave selection on index 2"
        );

        params
            .load_preset(1)
            .expect("preset selection should succeed");
        assert_eq!(
            params.preset_bank_snapshot().selected,
            1,
            "active runtime selection should move to index 1"
        );

        let restored = PumpParams::new();
        assert_eq!(
            restored.preset_bank_snapshot().selected,
            1,
            "new instance should restore persisted selected preset index"
        );
    });
    let _ = std::fs::remove_file(path);
}

#[test]
fn preset_create_rolls_back_for_create_write_and_rename_failures() {
    use super::preset_store::{with_test_persistence_failure, TestPersistenceFailure};

    for (label, failure, expected_message) in [
        (
            "create-failure",
            TestPersistenceFailure::CreateDirectory,
            "creating preset store directory",
        ),
        (
            "write-failure",
            TestPersistenceFailure::WriteTemporary,
            "writing temporary preset store",
        ),
        (
            "rename-failure",
            TestPersistenceFailure::RenameTemporary,
            "finalizing preset store",
        ),
    ] {
        let path = temp_preset_store_path(label);
        super::preset_store::with_test_persistence_path(path.clone(), || {
            let params = PumpParams::new();
            let before = params.preset_bank_snapshot();

            let result =
                with_test_persistence_failure(failure, || params.add_preset_from_current_state());
            assert!(matches!(
                result,
                Err(PresetMutationError::PersistenceFailed { ref message })
                    if message.contains(expected_message)
            ));
            assert_eq!(
                params.preset_bank_snapshot(),
                before,
                "failed create must roll back the in-memory bank"
            );
            assert!(params.preset_persistence_warning().is_some());

            let restored = PumpParams::new();
            assert_eq!(
                restored.preset_bank_snapshot(),
                before,
                "failed create must not appear after reload"
            );
        });
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(unix)]
#[test]
fn preset_create_rolls_back_when_preset_directory_is_unwritable() {
    use std::os::unix::fs::PermissionsExt;

    let path = temp_preset_store_path("unwritable-directory");
    let directory = path.with_extension("dir");
    std::fs::create_dir_all(&directory).expect("test directory should be created");
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500))
        .expect("test directory should become unwritable");
    let store_path = directory.join("preset-bank.bin");

    super::preset_store::with_test_persistence_path(store_path, || {
        let params = PumpParams::new();
        let before = params.preset_bank_snapshot();
        let result = params.add_preset_from_current_state();
        assert!(matches!(
            result,
            Err(PresetMutationError::PersistenceFailed { .. })
        ));
        assert_eq!(params.preset_bank_snapshot(), before);
        assert!(params.preset_persistence_warning().is_some());
    });

    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        .expect("test directory permissions should be restored");
    std::fs::remove_dir_all(directory).expect("test directory should be removed");
}

#[test]
fn preset_rename_overwrite_quick_slot_and_selection_roll_back_on_write_failure() {
    use super::preset_store::{with_test_persistence_failure, TestPersistenceFailure};

    let path = temp_preset_store_path("mutation-rollback");
    super::preset_store::with_test_persistence_path(path.clone(), || {
        let params = PumpParams::new();
        params.set_mix(0.22);
        params
            .add_preset_from_current_state()
            .expect("baseline preset should persist");
        let baseline = params.preset_bank_snapshot();

        let rename = with_test_persistence_failure(TestPersistenceFailure::WriteTemporary, || {
            params.rename_preset(1, "Unsaved rename")
        });
        assert!(matches!(
            rename,
            Err(PresetMutationError::PersistenceFailed { .. })
        ));
        assert_eq!(params.preset_bank_snapshot(), baseline);

        params.set_mix(0.81);
        let overwrite =
            with_test_persistence_failure(TestPersistenceFailure::WriteTemporary, || {
                params.save_current_state_by_name(&baseline.presets[1].name)
            });
        assert!(matches!(
            overwrite,
            Err(PresetMutationError::PersistenceFailed { .. })
        ));
        assert_eq!(params.preset_bank_snapshot(), baseline);

        let unsaved_curve = test_quick_slot_curve(0.14);
        let quick_slot =
            with_test_persistence_failure(TestPersistenceFailure::WriteTemporary, || {
                params.set_selected_quick_slot_curve(0, &unsaved_curve)
            });
        assert!(matches!(
            quick_slot,
            Err(PresetMutationError::PersistenceFailed { .. })
        ));
        assert_eq!(params.preset_bank_snapshot(), baseline);

        let mix_before_failed_load = params.mix();
        let selection =
            with_test_persistence_failure(TestPersistenceFailure::WriteTemporary, || {
                params.load_preset(0)
            });
        assert!(matches!(
            selection,
            Err(PresetMutationError::PersistenceFailed { .. })
        ));
        assert_eq!(params.preset_bank_snapshot(), baseline);
        assert!((params.mix() - mix_before_failed_load).abs() < 1.0e-6);

        let restored = PumpParams::new();
        assert_eq!(restored.preset_bank_snapshot(), baseline);

        params
            .rename_preset(1, "Saved rename")
            .expect("a subsequent successful write should recover");
        assert_eq!(params.preset_persistence_warning(), None);
        assert_eq!(
            PumpParams::new().preset_bank_snapshot().presets[1].name,
            "Saved rename"
        );
    });
    let _ = std::fs::remove_file(path);
}

#[test]
fn failed_final_rename_preserves_the_previous_durable_bank() {
    use super::preset_store::{with_test_persistence_failure, TestPersistenceFailure};

    let path = temp_preset_store_path("preserve-durable-bank");
    super::preset_store::with_test_persistence_path(path.clone(), || {
        let params = PumpParams::new();
        params
            .add_preset_from_current_state()
            .expect("baseline bank should persist");
        let baseline = params.preset_bank_snapshot();

        let result = with_test_persistence_failure(TestPersistenceFailure::RenameTemporary, || {
            params.rename_preset(1, "Unsaved rename")
        });
        assert!(matches!(
            result,
            Err(PresetMutationError::PersistenceFailed { .. })
        ));
        assert_eq!(params.preset_bank_snapshot(), baseline);
        assert_eq!(PumpParams::new().preset_bank_snapshot(), baseline);
    });
    let _ = std::fs::remove_file(path);
}

#[test]
fn set_preset_bank_preserves_user_presets_without_inserting_init() {
    let params = PumpParams::new();
    params
        .set_preset_bank(PumpPresetBank {
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
                    quick_slots: seeded_quick_shape_slots(),
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
                    quick_slots: seeded_quick_shape_slots(),
                },
            ],
        })
        .expect("preset bank should persist");

    let bank = params.preset_bank_snapshot();
    assert_eq!(bank.presets.len(), 2);
    assert_eq!(bank.selected, 1);
    assert_eq!(bank.presets[0].name, "Live A");
    assert!((bank.presets[0].mix - 0.11).abs() < 1.0e-6);
    assert!((bank.presets[0].depth - 1.0).abs() < 1.0e-6);
    assert_eq!(bank.presets[1].name, "Live B");
    assert!((bank.presets[1].mix - 0.77).abs() < 1.0e-6);
    assert!((bank.presets[1].depth - 1.0).abs() < 1.0e-6);
}

#[test]
fn save_by_name_overwrites_case_insensitive_match() {
    let params = PumpParams::new();
    params
        .add_preset_from_current_state()
        .expect("preset insertion should succeed");
    assert!(params.rename_preset(1, "Verse").is_ok());
    params.set_mix(0.12);

    let outcome = params.save_current_state_by_name(" verse ");
    assert_eq!(outcome, Ok(SavePresetOutcome::Overwritten { index: 1 }));
    let bank = params.preset_bank_snapshot();
    assert_eq!(bank.selected, 1);
    assert_eq!(bank.presets[1].name, "Verse");
    assert!((bank.presets[1].mix - 0.12).abs() < 1.0e-6);
    assert!((bank.presets[1].depth - 1.0).abs() < 1.0e-6);
}

#[test]
fn quick_slot_edits_switch_with_selected_preset() {
    let params = PumpParams::new();
    let curve_a = test_quick_slot_curve(0.02);
    let curve_b = test_quick_slot_curve(0.11);
    assert!(params.set_selected_quick_slot_curve(0, &curve_a).is_ok());

    let inserted = params
        .add_preset_from_current_state()
        .expect("preset insertion should succeed");
    assert_eq!(inserted, 1);
    assert!(params.set_selected_quick_slot_curve(0, &curve_b).is_ok());

    params.load_preset(0).expect("preset load should succeed");
    assert_eq!(
        params
            .selected_quick_slot_curve(0)
            .expect("slot should exist"),
        curve_a
    );

    params.load_preset(1).expect("preset load should succeed");
    assert_eq!(
        params
            .selected_quick_slot_curve(0)
            .expect("slot should exist"),
        curve_b
    );
}

#[test]
fn save_by_name_preserves_selected_preset_quick_slots() {
    let params = PumpParams::new();
    let slot_curve = test_quick_slot_curve(0.05);
    assert!(params.set_selected_quick_slot_curve(2, &slot_curve).is_ok());

    let outcome = params.save_current_state_by_name("Transients");
    assert_eq!(outcome, Ok(SavePresetOutcome::Created { index: 1 }));
    let bank = params.preset_bank_snapshot();
    assert_eq!(bank.presets[1].quick_slots[2].curve, slot_curve);
}

#[test]
fn save_by_name_creates_when_no_match_exists() {
    let params = PumpParams::new();
    params.set_mix(0.77);
    let outcome = params.save_current_state_by_name("Hook");
    assert_eq!(outcome, Ok(SavePresetOutcome::Created { index: 1 }));
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
    assert!(params.rename_preset(0, "Init2").is_ok());
    assert_eq!(
        params.save_current_state_by_name("Init2"),
        Ok(SavePresetOutcome::Overwritten { index: 0 })
    );
    let bank = params.preset_bank_snapshot();
    assert_eq!(bank.presets[0].name, "Init2");
    assert!((bank.presets[0].mix - 0.61).abs() < 1.0e-6);
    assert!(!bank.presets[0].is_read_only);
}

#[cfg(feature = "vst3")]
#[test]
fn vst3_mapping_resolves_to_shared_clap_ids() {
    assert_eq!(
        clap_id_from_vst3_param_id(PARAM_MIX_NUM),
        Some(PARAM_MIX_ID)
    );
    assert_eq!(
        clap_id_from_vst3_param_id(PARAM_PHASE_OFFSET_NUM),
        Some(PARAM_PHASE_OFFSET_ID)
    );
    assert_eq!(
        clap_id_from_vst3_param_id(PARAM_OUTPUT_GAIN_NUM),
        Some(PARAM_OUTPUT_GAIN_ID)
    );
    assert_eq!(
        clap_id_from_vst3_param_id(PARAM_SYNC_DIVISION_NUM),
        Some(PARAM_SYNC_DIVISION_ID)
    );
    assert_eq!(clap_id_from_vst3_param_id(999), None);
}

#[cfg(feature = "vst3")]
#[test]
fn vst3_info_and_text_conversions_share_param_rules() {
    let mix_info = vst3_param_info_for_index(0).expect("mix info should exist");
    assert_eq!(mix_info.id, PARAM_MIX_NUM);
    assert_eq!(mix_info.title, "Mix");
    assert_eq!(mix_info.units, "%");

    let division_info = vst3_param_info_for_index(3).expect("division info should exist");
    assert_eq!(division_info.id, PARAM_SYNC_DIVISION_NUM);
    assert_eq!(division_info.step_count, MAX_SYNC_DIVISION as i32);

    let mix_text = format_plain_value_text(PARAM_MIX_ID, 0.5).expect("mix text");
    assert_eq!(mix_text, "50%");
    assert_eq!(parse_plain_value_text(PARAM_MIX_ID, "50%"), Some(0.5));

    let output_text = format_plain_value_text(PARAM_OUTPUT_GAIN_ID, -3.5).expect("output text");
    assert_eq!(output_text, "-3.5 dB");
    assert_eq!(
        parse_plain_value_text(PARAM_OUTPUT_GAIN_ID, "-3.5 dB"),
        Some(-3.5)
    );

    let division_text =
        format_plain_value_text(PARAM_SYNC_DIVISION_ID, 7.0).expect("division text");
    assert_eq!(division_text, "2 Bars");
    assert_eq!(
        parse_plain_value_text(PARAM_SYNC_DIVISION_ID, "2 Bars"),
        Some(7.0)
    );
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
        Err(PresetMutationError::CapacityReached)
    );
}
