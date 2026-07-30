use super::*;

const MIN_CURVE_BYTES: usize = 2 * 8 + 4;
const MIN_ENCODED_CURVE_BYTES: usize = 4 + MIN_CURVE_BYTES;
const PHASE_METADATA_MAGIC: u32 = u32::from_le_bytes(*b"PHAS");
/// Encode all parameter state including curve table into bytes.
pub fn encode_state_payload(params: &PumpParams) -> Vec<u8> {
    let editable = params
        .sound_state_snapshot(params.active_sound())
        .editable_curve;
    let node_count = editable.nodes.len().min(MAX_EDITABLE_NODES);
    let segment_count = node_count.saturating_sub(1);
    let bank = params.preset_bank_snapshot();
    let mut payload = Vec::with_capacity(64 + node_count * 8 + segment_count * 4);

    payload.extend_from_slice(STATE_MAGIC);
    payload.extend_from_slice(&STATE_VERSION.to_le_bytes());
    payload.extend_from_slice(&params.mix().to_le_bytes());
    payload.extend_from_slice(&params.depth_db().to_le_bytes());
    payload.extend_from_slice(&params.floor_db().to_le_bytes());
    payload.extend_from_slice(&params.phase_offset().to_le_bytes());
    payload.extend_from_slice(&params.output_gain_db().to_le_bytes());
    payload.extend_from_slice(&(params.sync_division() as f32).to_le_bytes());
    payload.extend_from_slice(&(node_count as u32).to_le_bytes());

    for node in editable.nodes.iter().take(node_count) {
        payload.extend_from_slice(&node.x.to_le_bytes());
        payload.extend_from_slice(&node.y.to_le_bytes());
    }
    for segment in editable.segments.iter().take(segment_count) {
        payload.extend_from_slice(&segment.tension.to_le_bytes());
    }
    encode_phase_metadata(&mut payload, &editable);
    payload.extend_from_slice(&(bank.selected as u32).to_le_bytes());
    payload.extend_from_slice(&(bank.presets.len() as u32).to_le_bytes());
    for (index, preset) in bank.presets.iter().enumerate() {
        encode_preset(&mut payload, preset, index);
    }
    payload.extend_from_slice(&(params.trigger_mode() as f32).to_le_bytes());
    payload.extend_from_slice(&params.smooth().to_le_bytes());
    payload.extend_from_slice(&(params.mode() as f32).to_le_bytes());
    payload.extend_from_slice(&params.bypass_value().to_le_bytes());
    payload.extend_from_slice(&params.swing().to_le_bytes());

    payload.extend_from_slice(&(params.active_sound().index() as u32).to_le_bytes());
    for side in [SoundSide::A, SoundSide::B] {
        encode_sound_state(&mut payload, &params.sound_state_snapshot(side));
    }
    for side in [SoundSide::A, SoundSide::B] {
        encode_sound_state(&mut payload, &params.stored_sound_state_snapshot(side));
    }
    payload.extend_from_slice(&(params.timing_mode() as u32).to_le_bytes());
    payload.extend_from_slice(&params.free_rate_hz().to_le_bytes());

    payload
}

/// Decode parameter state payload and apply it to shared params.
pub fn decode_state_payload(params: &PumpParams, payload: &[u8]) -> Result<(), &'static str> {
    if payload.len() == legacy_payload_len() {
        return decode_legacy_state_payload(params, payload);
    }

    let mut cursor = Cursor::new(payload);
    let Some(magic) = read_u32(&mut cursor) else {
        return Err("invalid state header");
    };
    if magic != u32::from_le_bytes(*STATE_MAGIC) {
        return Err("unknown state payload format");
    }
    let Some(version) = read_u32(&mut cursor) else {
        return Err("invalid state version");
    };
    if !(2..=STATE_VERSION).contains(&version) {
        return Err("unsupported state payload version");
    }

    let Some(mix) = read_f32(&mut cursor) else {
        return Err("invalid mix field");
    };
    let Some(depth_field) = read_f32(&mut cursor) else {
        return Err("invalid depth field");
    };
    let floor_db = if version >= 7 {
        let Some(floor_db) = read_f32(&mut cursor) else {
            return Err("invalid floor field");
        };
        floor_db
    } else {
        DEFAULT_FLOOR_DB
    };
    let depth_db = if version >= 7 {
        depth_field
    } else {
        DEFAULT_DEPTH_DB
    };
    let Some(phase_offset) = read_f32(&mut cursor) else {
        return Err("invalid phase offset field");
    };
    let Some(output_gain_db) = read_f32(&mut cursor) else {
        return Err("invalid output gain field");
    };
    let Some(sync_division) = read_f32(&mut cursor) else {
        return Err("invalid sync division field");
    };
    let Some(node_count) = read_u32(&mut cursor).map(|count| count as usize) else {
        return Err("invalid node count");
    };
    let editable_curve = decode_curve(&mut cursor, node_count, version >= 6)?;

    let preset_bank = if version >= 3 {
        decode_preset_bank(&mut cursor, version)?
    } else {
        PumpPresetBank {
            selected: 0,
            presets: vec![PumpPreset {
                name: DEFAULT_PRESET_NAME.to_string(),
                is_read_only: false,
                is_favorite: false,
                mix,
                depth: DEFAULT_DEPTH,
                depth_db,
                floor_db,
                phase_offset,
                output_gain_db,
                sync_division: clamp_sync_division(sync_division),
                trigger_mode: DEFAULT_TRIGGER_MODE,
                smooth: DEFAULT_SMOOTH,
                mode: PROCESSING_MODE_CLASSIC,
                swing: DEFAULT_SWING,
                timing_mode: DEFAULT_TIMING_MODE,
                free_rate_hz: DEFAULT_FREE_RATE_HZ,
                editable_curve: editable_curve.clone(),
                quick_slots: seeded_quick_shape_slots(),
            }],
        }
    };

    let trigger_mode = if version >= 8 {
        let Some(trigger_mode) = read_f32(&mut cursor) else {
            return Err("invalid trigger mode field");
        };
        trigger_mode
    } else {
        DEFAULT_TRIGGER_MODE as f32
    };
    let smooth = if version >= 10 {
        let Some(smooth) = read_f32(&mut cursor) else {
            return Err("invalid smooth field");
        };
        smooth
    } else {
        DEFAULT_SMOOTH
    };
    let mode = if version >= 11 {
        let Some(mode) = read_f32(&mut cursor) else {
            return Err("invalid processing mode field");
        };
        clamp_processing_mode(mode)
    } else {
        PROCESSING_MODE_CLASSIC
    };
    let bypass = if version >= 12 {
        let Some(bypass) = read_f32(&mut cursor) else {
            return Err("invalid bypass field");
        };
        if !bypass.is_finite() {
            return Err("invalid bypass field");
        }
        bypass
    } else {
        BYPASS_ACTIVE_VALUE
    };
    let swing = if version >= 13 {
        let Some(swing) = read_f32(&mut cursor) else {
            return Err("invalid swing field");
        };
        swing
    } else {
        DEFAULT_SWING
    };
    if version >= 14
        && ![
            mix,
            depth_field,
            floor_db,
            phase_offset,
            output_gain_db,
            sync_division,
            trigger_mode,
            smooth,
            bypass,
            swing,
        ]
        .into_iter()
        .all(f32::is_finite)
    {
        return Err("invalid A/B scalar field");
    }
    let (active_sound, sound_states, stored_sound_states) = if version >= 14 {
        let Some(active_sound) = read_u32(&mut cursor).map(|value| {
            if value == SoundSide::B.index() as u32 {
                SoundSide::B
            } else {
                SoundSide::A
            }
        }) else {
            return Err("invalid active sound field");
        };
        let a = decode_sound_state(&mut cursor, version)?;
        let b = decode_sound_state(&mut cursor, version)?;
        let working = [a, b];
        let stored = if version >= 16 {
            [
                decode_sound_state(&mut cursor, version)?,
                decode_sound_state(&mut cursor, version)?,
            ]
        } else {
            working.clone()
        };
        (active_sound, working, stored)
    } else {
        let legacy = PumpSoundState {
            mix,
            depth_db,
            floor_db,
            phase_offset,
            output_gain_db,
            sync_division: clamp_sync_division(sync_division),
            trigger_mode: clamp_trigger_mode(trigger_mode),
            smooth,
            mode,
            swing,
            timing_mode: DEFAULT_TIMING_MODE,
            free_rate_hz: DEFAULT_FREE_RATE_HZ,
            editable_curve: editable_curve.clone(),
            quick_slots: preset_bank
                .presets
                .get(preset_bank.selected)
                .map(|preset| preset.quick_slots.clone())
                .unwrap_or_else(seeded_quick_shape_slots),
        };
        (
            SoundSide::A,
            [legacy.clone(), legacy.clone()],
            [legacy.clone(), legacy],
        )
    };
    let (timing_mode, free_rate_hz) = if version >= 15 {
        let Some(timing_mode) = read_u32(&mut cursor) else {
            return Err("invalid timing mode field");
        };
        let Some(free_rate_hz) = read_f32(&mut cursor) else {
            return Err("invalid free rate field");
        };
        if !free_rate_hz.is_finite() {
            return Err("invalid free rate field");
        }
        (
            clamp_timing_mode(timing_mode as f32),
            clamp_free_rate_hz(free_rate_hz),
        )
    } else {
        (DEFAULT_TIMING_MODE, DEFAULT_FREE_RATE_HZ)
    };
    if cursor.position() != payload.len() as u64 {
        return Err("unexpected trailing state bytes");
    }

    params.set_mix(mix);
    params.set_depth_db(depth_db);
    params.set_floor_db(floor_db);
    params.set_phase_offset(phase_offset);
    params.set_output_gain_db(output_gain_db);
    params.set_sync_division(sync_division);
    params.set_trigger_mode(trigger_mode);
    params.set_smooth(smooth);
    params.set_mode(mode as f32);
    params.set_bypass(bypass);
    params.set_swing(swing);
    params.set_timing_mode(timing_mode as f32);
    params.set_free_rate_hz(free_rate_hz);
    params.set_editable_curve_preserving_phase(&editable_curve);
    params.set_preset_bank_without_persistence(preset_bank);
    params.set_sound_states_with_references_without_persistence(
        active_sound,
        sound_states,
        stored_sound_states,
    );

    Ok(())
}

fn encode_sound_state(payload: &mut Vec<u8>, state: &PumpSoundState) {
    payload.extend_from_slice(&state.mix.to_le_bytes());
    payload.extend_from_slice(&state.depth_db.to_le_bytes());
    payload.extend_from_slice(&state.floor_db.to_le_bytes());
    payload.extend_from_slice(&state.phase_offset.to_le_bytes());
    payload.extend_from_slice(&state.output_gain_db.to_le_bytes());
    payload.extend_from_slice(&(state.sync_division as u32).to_le_bytes());
    payload.extend_from_slice(&(state.trigger_mode as u32).to_le_bytes());
    payload.extend_from_slice(&state.smooth.to_le_bytes());
    payload.extend_from_slice(&(state.mode as u32).to_le_bytes());
    payload.extend_from_slice(&state.swing.to_le_bytes());
    encode_curve(payload, &state.editable_curve);
    payload.extend_from_slice(&(state.quick_slots.len() as u32).to_le_bytes());
    for slot in &state.quick_slots {
        encode_curve(payload, &slot.curve);
    }
    payload.extend_from_slice(&(state.timing_mode as u32).to_le_bytes());
    payload.extend_from_slice(&state.free_rate_hz.to_le_bytes());
}

fn decode_sound_state(
    cursor: &mut Cursor<&[u8]>,
    version: u32,
) -> Result<PumpSoundState, &'static str> {
    let Some(mix) = read_f32(cursor) else {
        return Err("invalid A/B mix field");
    };
    let Some(depth_db) = read_f32(cursor) else {
        return Err("invalid A/B depth field");
    };
    let Some(floor_db) = read_f32(cursor) else {
        return Err("invalid A/B floor field");
    };
    let Some(phase_offset) = read_f32(cursor) else {
        return Err("invalid A/B phase field");
    };
    let Some(output_gain_db) = read_f32(cursor) else {
        return Err("invalid A/B output field");
    };
    let Some(sync_division) = read_u32(cursor) else {
        return Err("invalid A/B sync field");
    };
    let Some(trigger_mode) = read_u32(cursor) else {
        return Err("invalid A/B trigger field");
    };
    let Some(smooth) = read_f32(cursor) else {
        return Err("invalid A/B smooth field");
    };
    let Some(mode) = read_u32(cursor) else {
        return Err("invalid A/B mode field");
    };
    let Some(swing) = read_f32(cursor) else {
        return Err("invalid A/B swing field");
    };
    let Some(node_count) = read_u32(cursor).map(|count| count as usize) else {
        return Err("invalid A/B node count");
    };
    let editable_curve = decode_curve(cursor, node_count, true)?;
    let Some(quick_count) = read_u32(cursor).map(|count| count as usize) else {
        return Err("invalid A/B quick slot count");
    };
    if quick_count != QUICK_SLOT_COUNT {
        return Err("invalid A/B quick slot count bounds");
    }
    let mut quick_slots = Vec::with_capacity(quick_count);
    for _ in 0..quick_count {
        let Some(node_count) = read_u32(cursor).map(|count| count as usize) else {
            return Err("invalid A/B quick slot node count");
        };
        quick_slots.push(QuickShapeSlot {
            curve: decode_curve(cursor, node_count, true)?,
        });
    }
    let (timing_mode, free_rate_hz) = if version >= 15 {
        let Some(timing_mode) = read_u32(cursor) else {
            return Err("invalid A/B timing mode field");
        };
        let Some(free_rate_hz) = read_f32(cursor) else {
            return Err("invalid A/B free rate field");
        };
        if !free_rate_hz.is_finite() {
            return Err("invalid A/B free rate field");
        }
        (
            clamp_timing_mode(timing_mode as f32),
            clamp_free_rate_hz(free_rate_hz),
        )
    } else {
        (DEFAULT_TIMING_MODE, DEFAULT_FREE_RATE_HZ)
    };
    if ![
        mix,
        depth_db,
        floor_db,
        phase_offset,
        output_gain_db,
        smooth,
        swing,
    ]
    .into_iter()
    .all(f32::is_finite)
    {
        return Err("invalid A/B scalar field");
    }
    Ok(PumpSoundState {
        mix,
        depth_db,
        floor_db,
        phase_offset,
        output_gain_db,
        sync_division: (sync_division as usize).min(MAX_SYNC_DIVISION as usize),
        trigger_mode: clamp_trigger_mode(trigger_mode as f32),
        smooth,
        mode: clamp_processing_mode(mode as f32),
        swing,
        timing_mode,
        free_rate_hz,
        editable_curve: editable_curve.normalized(),
        quick_slots,
    })
}

fn encode_preset(payload: &mut Vec<u8>, preset: &PumpPreset, index: usize) {
    let name = sanitize_preset_name(&preset.name, index);
    payload.extend_from_slice(&(name.len() as u32).to_le_bytes());
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(&preset.mix.to_le_bytes());
    payload.extend_from_slice(&preset.depth_db.to_le_bytes());
    payload.extend_from_slice(&preset.floor_db.to_le_bytes());
    payload.extend_from_slice(&preset.phase_offset.to_le_bytes());
    payload.extend_from_slice(&preset.output_gain_db.to_le_bytes());
    payload.extend_from_slice(&(preset.sync_division as u32).to_le_bytes());
    payload.push(u8::from(preset.is_read_only));
    encode_curve(payload, &preset.editable_curve);
    payload.extend_from_slice(&(preset.quick_slots.len() as u32).to_le_bytes());
    for slot in &preset.quick_slots {
        encode_curve(payload, &slot.curve);
    }
    payload.extend_from_slice(&(preset.trigger_mode as u32).to_le_bytes());
    payload.extend_from_slice(&preset.smooth.to_le_bytes());
    payload.extend_from_slice(&(clamp_processing_mode(preset.mode as f32) as u32).to_le_bytes());
    payload.push(u8::from(preset.is_favorite));
    payload.extend_from_slice(&preset.swing.to_le_bytes());
    payload.extend_from_slice(&(preset.timing_mode as u32).to_le_bytes());
    payload.extend_from_slice(&preset.free_rate_hz.to_le_bytes());
}

fn encode_curve(payload: &mut Vec<u8>, curve: &EditableCurve) {
    let normalized_curve = curve.clone().normalized();
    let node_count = normalized_curve.nodes.len().min(MAX_EDITABLE_NODES);
    payload.extend_from_slice(&(node_count as u32).to_le_bytes());
    for node in normalized_curve.nodes.iter().take(node_count) {
        payload.extend_from_slice(&node.x.to_le_bytes());
        payload.extend_from_slice(&node.y.to_le_bytes());
    }
    for segment in normalized_curve
        .segments
        .iter()
        .take(node_count.saturating_sub(1))
    {
        payload.extend_from_slice(&segment.tension.to_le_bytes());
    }
    encode_phase_metadata(payload, &normalized_curve);
}

fn encode_phase_metadata(payload: &mut Vec<u8>, curve: &EditableCurve) {
    let Some(source) = curve.phase_source.as_deref() else {
        return;
    };
    payload.extend_from_slice(&PHASE_METADATA_MAGIC.to_le_bytes());
    payload.extend_from_slice(&curve.phase_offset.to_le_bytes());
    let source = source.clone().normalized();
    let node_count = source.nodes.len().min(MAX_EDITABLE_NODES);
    payload.extend_from_slice(&(node_count as u32).to_le_bytes());
    for node in source.nodes.iter().take(node_count) {
        payload.extend_from_slice(&node.x.to_le_bytes());
        payload.extend_from_slice(&node.y.to_le_bytes());
    }
    for segment in source.segments.iter().take(node_count.saturating_sub(1)) {
        payload.extend_from_slice(&segment.tension.to_le_bytes());
    }
}

fn decode_curve(
    cursor: &mut Cursor<&[u8]>,
    node_count: usize,
    with_phase_metadata: bool,
) -> Result<EditableCurve, &'static str> {
    if !(2..=MAX_EDITABLE_NODES).contains(&node_count) {
        return Err("invalid node count bounds");
    }
    let required_bytes = node_count * 8 + node_count.saturating_sub(1) * 4;
    if remaining_bytes(cursor) < required_bytes {
        return Err("invalid curve byte count");
    }
    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let Some(x) = read_f32(cursor) else {
            return Err("invalid curve node x");
        };
        let Some(y) = read_f32(cursor) else {
            return Err("invalid curve node y");
        };
        nodes.push(CurveNode { x, y });
    }
    let mut segments = Vec::with_capacity(node_count.saturating_sub(1));
    for _ in 0..node_count.saturating_sub(1) {
        let Some(tension) = read_f32(cursor) else {
            return Err("invalid curve segment");
        };
        segments.push(CurveSegment { tension });
    }
    let mut curve = EditableCurve {
        nodes,
        segments,
        ..EditableCurve::default()
    };
    if with_phase_metadata {
        let marker_position = cursor.position();
        let marker = read_u32(cursor).unwrap_or_default();
        if marker == PHASE_METADATA_MAGIC {
            let Some(phase_offset) = read_f32(cursor) else {
                return Err("invalid curve phase offset");
            };
            let Some(source_node_count) = read_u32(cursor).map(|count| count as usize) else {
                return Err("invalid curve phase source node count");
            };
            let source = decode_curve(cursor, source_node_count, false)?;
            curve.phase_source = Some(Box::new(source));
            curve.phase_offset = phase_offset;
        } else {
            cursor.set_position(marker_position);
        }
    }
    Ok(curve)
}

fn decode_preset_bank(
    cursor: &mut Cursor<&[u8]>,
    version: u32,
) -> Result<PumpPresetBank, &'static str> {
    let Some(selected) = read_u32(cursor).map(|value| value as usize) else {
        return Err("invalid preset selected index");
    };
    let Some(count) = read_u32(cursor).map(|value| value as usize) else {
        return Err("invalid preset count");
    };
    if count == 0 || count > MAX_PRESETS {
        return Err("invalid preset count bounds");
    }
    if remaining_bytes(cursor) < count * 4 {
        return Err("invalid preset count byte length");
    }
    let mut presets = Vec::with_capacity(count);
    for index in 0..count {
        let Some(name_len) = read_u32(cursor).map(|value| value as usize) else {
            return Err("invalid preset name length");
        };
        if name_len == 0 || name_len > 256 {
            return Err("invalid preset name length bounds");
        }
        if remaining_bytes(cursor) < name_len {
            return Err("invalid preset name byte length");
        }
        let mut name_bytes = vec![0_u8; name_len];
        std::io::Read::read_exact(cursor, &mut name_bytes).map_err(|_| "invalid preset name")?;
        let raw_name = std::str::from_utf8(&name_bytes).map_err(|_| "invalid preset name utf8")?;
        let Some(mix) = read_f32(cursor) else {
            return Err("invalid preset mix");
        };
        let Some(depth_field) = read_f32(cursor) else {
            return Err("invalid preset depth");
        };
        let (depth, floor_db) = if version >= 7 {
            let Some(floor_db) = read_f32(cursor) else {
                return Err("invalid preset floor");
            };
            (depth_field, floor_db)
        } else {
            (DEFAULT_DEPTH_DB, DEFAULT_FLOOR_DB)
        };
        let Some(phase_offset) = read_f32(cursor) else {
            return Err("invalid preset phase offset");
        };
        let Some(output_gain_db) = read_f32(cursor) else {
            return Err("invalid preset output gain");
        };
        let Some(sync_division) = read_u32(cursor).map(|value| value as usize) else {
            return Err("invalid preset sync division");
        };
        if version >= 4 {
            let Some(flag) = read_u8(cursor) else {
                return Err("invalid preset read-only flag");
            };
            let _ = flag;
        }
        let Some(node_count) = read_u32(cursor).map(|value| value as usize) else {
            return Err("invalid preset node count");
        };
        let editable_curve = decode_curve(cursor, node_count, version >= 6)?;
        let quick_slots = if version >= 5 {
            decode_quick_slots(cursor, version >= 6)?
        } else {
            seeded_quick_shape_slots()
        };
        let trigger_mode = if version >= 8 {
            let Some(trigger_mode) = read_u32(cursor).map(|value| value as usize) else {
                return Err("invalid preset trigger mode");
            };
            trigger_mode
        } else {
            DEFAULT_TRIGGER_MODE
        };
        let smooth = if version >= 10 {
            read_f32(cursor).ok_or("invalid preset smooth")?
        } else {
            DEFAULT_SMOOTH
        };
        let mode = if version >= 11 {
            let Some(mode) = read_u32(cursor).map(|value| value as f32) else {
                return Err("invalid preset processing mode");
            };
            clamp_processing_mode(mode)
        } else {
            PROCESSING_MODE_CLASSIC
        };
        let is_favorite = if version >= 9 {
            read_u8(cursor).ok_or("invalid preset favorite flag")? != 0
        } else {
            false
        };
        let swing = if version >= 13 {
            read_f32(cursor).ok_or("invalid preset swing")?
        } else {
            DEFAULT_SWING
        };
        let (timing_mode, free_rate_hz) = if version >= 15 {
            let timing_mode = read_u32(cursor).ok_or("invalid preset timing mode")?;
            let free_rate_hz = read_f32(cursor).ok_or("invalid preset free rate")?;
            if !free_rate_hz.is_finite() {
                return Err("invalid preset free rate");
            }
            (
                clamp_timing_mode(timing_mode as f32),
                clamp_free_rate_hz(free_rate_hz),
            )
        } else {
            (DEFAULT_TIMING_MODE, DEFAULT_FREE_RATE_HZ)
        };
        presets.push(PumpPreset {
            name: sanitize_preset_name(raw_name, index),
            is_read_only: false,
            is_favorite,
            mix,
            depth: (depth / MAX_DEPTH_DB).clamp(0.0, 1.0),
            depth_db: depth,
            floor_db,
            phase_offset,
            output_gain_db,
            sync_division: sync_division.min(MAX_SYNC_DIVISION as usize),
            trigger_mode: clamp_trigger_mode(trigger_mode as f32),
            smooth,
            mode,
            swing,
            timing_mode,
            free_rate_hz,
            editable_curve: editable_curve.normalized(),
            quick_slots,
        });
    }
    Ok(PumpPresetBank {
        selected: selected.min(count.saturating_sub(1)),
        presets,
    })
}

fn decode_quick_slots(
    cursor: &mut Cursor<&[u8]>,
    with_phase_metadata: bool,
) -> Result<Vec<QuickShapeSlot>, &'static str> {
    let Some(count) = read_u32(cursor).map(|value| value as usize) else {
        return Err("invalid preset quick slot count");
    };
    if count != QUICK_SLOT_COUNT {
        return Err("invalid preset quick slot count bounds");
    }
    if remaining_bytes(cursor) < count * MIN_ENCODED_CURVE_BYTES {
        return Err("invalid preset quick slot count byte length");
    }
    let mut slots = Vec::with_capacity(count);
    for _ in 0..count {
        let Some(node_count) = read_u32(cursor).map(|value| value as usize) else {
            return Err("invalid preset quick slot node count");
        };
        let curve = decode_curve(cursor, node_count, with_phase_metadata)?;
        slots.push(QuickShapeSlot { curve });
    }
    Ok(slots)
}

fn legacy_payload_len() -> usize {
    4 * (5 + CURVE_TABLE_LEN)
}

fn decode_legacy_state_payload(params: &PumpParams, payload: &[u8]) -> Result<(), &'static str> {
    if payload.len() != legacy_payload_len() {
        return Err("invalid pump state payload length");
    }

    let mut cursor = Cursor::new(payload);
    let Some(mix) = read_f32(&mut cursor) else {
        return Err("invalid mix field");
    };
    let Some(_legacy_depth) = read_f32(&mut cursor) else {
        return Err("invalid depth field");
    };
    let Some(phase_offset) = read_f32(&mut cursor) else {
        return Err("invalid phase offset field");
    };
    let Some(output_gain_db) = read_f32(&mut cursor) else {
        return Err("invalid output gain field");
    };
    let Some(sync_division) = read_f32(&mut cursor) else {
        return Err("invalid sync division field");
    };

    let mut curve = [1.0_f32; CURVE_TABLE_LEN];
    for sample in &mut curve {
        let Some(value) = read_f32(&mut cursor) else {
            return Err("invalid curve sample");
        };
        *sample = value;
    }

    params.set_mix(mix);
    params.set_depth_db(DEFAULT_DEPTH_DB);
    params.set_floor_db(DEFAULT_FLOOR_DB);
    params.set_phase_offset(phase_offset);
    params.set_output_gain_db(output_gain_db);
    params.set_sync_division(sync_division);
    params.set_trigger_mode(DEFAULT_TRIGGER_MODE as f32);
    params.set_smooth(DEFAULT_SMOOTH);
    params.set_mode(PROCESSING_MODE_CLASSIC as f32);
    params.set_swing(DEFAULT_SWING);
    params.set_curve(&curve);
    let legacy_state = PumpSoundState {
        mix: params.mix(),
        depth_db: params.depth_db(),
        floor_db: params.floor_db(),
        phase_offset: params.phase_offset(),
        output_gain_db: params.output_gain_db(),
        sync_division: params.sync_division(),
        trigger_mode: DEFAULT_TRIGGER_MODE,
        smooth: DEFAULT_SMOOTH,
        mode: PROCESSING_MODE_CLASSIC,
        swing: DEFAULT_SWING,
        timing_mode: DEFAULT_TIMING_MODE,
        free_rate_hz: DEFAULT_FREE_RATE_HZ,
        editable_curve: params.editable_curve_snapshot(),
        quick_slots: seeded_quick_shape_slots(),
    };
    params.set_preset_bank_without_persistence(PumpPresetBank {
        selected: 0,
        presets: vec![PumpPreset {
            name: DEFAULT_PRESET_NAME.to_string(),
            is_read_only: false,
            is_favorite: false,
            mix: params.mix(),
            depth: params.depth(),
            depth_db: params.depth_db(),
            floor_db: params.floor_db(),
            phase_offset: params.phase_offset(),
            output_gain_db: params.output_gain_db(),
            sync_division: params.sync_division(),
            trigger_mode: DEFAULT_TRIGGER_MODE,
            smooth: DEFAULT_SMOOTH,
            mode: PROCESSING_MODE_CLASSIC,
            swing: DEFAULT_SWING,
            timing_mode: DEFAULT_TIMING_MODE,
            free_rate_hz: DEFAULT_FREE_RATE_HZ,
            editable_curve: params.editable_curve_snapshot(),
            quick_slots: seeded_quick_shape_slots(),
        }],
    });
    params.set_sound_states_without_persistence(SoundSide::A, [legacy_state.clone(), legacy_state]);

    Ok(())
}

fn read_f32(cursor: &mut Cursor<&[u8]>) -> Option<f32> {
    let mut bytes = [0_u8; 4];
    std::io::Read::read_exact(cursor, &mut bytes).ok()?;
    Some(f32::from_le_bytes(bytes))
}

fn read_u32(cursor: &mut Cursor<&[u8]>) -> Option<u32> {
    let mut bytes = [0_u8; 4];
    std::io::Read::read_exact(cursor, &mut bytes).ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn read_u8(cursor: &mut Cursor<&[u8]>) -> Option<u8> {
    let mut bytes = [0_u8; 1];
    std::io::Read::read_exact(cursor, &mut bytes).ok()?;
    Some(bytes[0])
}

fn remaining_bytes(cursor: &Cursor<&[u8]>) -> usize {
    cursor
        .get_ref()
        .len()
        .saturating_sub(cursor.position() as usize)
}
