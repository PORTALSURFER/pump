use super::*;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(test)]
use std::{cell::RefCell, panic::AssertUnwindSafe};

const PRESET_STORE_MAGIC: &[u8; 4] = b"PPBK";
const PRESET_STORE_VERSION: u32 = 2;
const PRESET_STORE_PATH_ENV: &str = "PUMP_PRESET_BANK_PATH";
const PRESET_STORE_FILE_NAME: &str = "preset-bank.bin";

#[cfg(test)]
thread_local! {
    static TEST_PERSISTENCE_PATH_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Run one closure with a thread-local preset-store path override enabled.
///
/// This helper keeps persistence tests isolated without touching process-global
/// environment variables.
#[cfg(test)]
pub(crate) fn with_test_persistence_path<R>(path: PathBuf, f: impl FnOnce() -> R) -> R {
    TEST_PERSISTENCE_PATH_OVERRIDE.with(|slot| {
        let previous = slot.replace(Some(path));
        let result = std::panic::catch_unwind(AssertUnwindSafe(f));
        slot.replace(previous);
        match result {
            Ok(value) => value,
            Err(error) => std::panic::resume_unwind(error),
        }
    })
}

/// Load the persisted preset bank from disk when available.
///
/// Returns `Ok(None)` when persistence is disabled or no store file exists.
pub(crate) fn load_persisted_preset_bank() -> Result<Option<PumpPresetBank>, String> {
    let Some(path) = persistence_file_path() else {
        return Ok(None);
    };
    if !path.is_file() {
        return Ok(None);
    }
    let payload = fs::read(&path)
        .map_err(|error| format!("failed to read preset store `{}`: {error}", path.display()))?;
    decode_preset_bank_payload(&payload).map(Some)
}

/// Persist one full preset bank snapshot to disk.
///
/// This helper uses a replace-on-rename write strategy to avoid partial files.
pub(crate) fn persist_preset_bank(bank: &PumpPresetBank) -> Result<(), String> {
    let Some(path) = persistence_file_path() else {
        return Ok(());
    };
    save_preset_bank_to_path(&path, bank)
}

fn persistence_file_path() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_PERSISTENCE_PATH_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return Some(path);
    }
    if cfg!(test) && std::env::var_os(PRESET_STORE_PATH_ENV).is_none() {
        return None;
    }
    if let Some(path) = std::env::var_os(PRESET_STORE_PATH_ENV).filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(path));
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(base) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
            return Some(
                PathBuf::from(base)
                    .join("PORTALSURFER")
                    .join("Pump")
                    .join(PRESET_STORE_FILE_NAME),
            );
        }
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(base) = std::env::var_os("XDG_STATE_HOME").filter(|value| !value.is_empty()) {
            return Some(
                PathBuf::from(base)
                    .join("pump")
                    .join(PRESET_STORE_FILE_NAME),
            );
        }
        if let Some(base) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
            return Some(
                PathBuf::from(base)
                    .join("pump")
                    .join(PRESET_STORE_FILE_NAME),
            );
        }
        std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(|base| {
                PathBuf::from(base)
                    .join(".config")
                    .join("pump")
                    .join(PRESET_STORE_FILE_NAME)
            })
    }
}

fn save_preset_bank_to_path(path: &Path, bank: &PumpPresetBank) -> Result<(), String> {
    let payload = encode_preset_bank_payload(bank);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create preset store directory `{}`: {error}",
                parent.display()
            )
        })?;
    }
    let temp_path = temporary_store_path(path);
    fs::write(&temp_path, payload).map_err(|error| {
        format!(
            "failed to write temporary preset store `{}`: {error}",
            temp_path.display()
        )
    })?;
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(path);
        if let Err(retry_error) = fs::rename(&temp_path, path) {
            let _ = fs::remove_file(&temp_path);
            return Err(format!(
                "failed to finalize preset store `{}` after `{}`: {retry_error}",
                path.display(),
                error
            ));
        }
    }
    Ok(())
}

fn temporary_store_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| PRESET_STORE_FILE_NAME.to_string());
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    path.with_file_name(format!("{file_name}.tmp-{pid}-{stamp}"))
}

fn encode_preset_bank_payload(bank: &PumpPresetBank) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(PRESET_STORE_MAGIC);
    payload.extend_from_slice(&PRESET_STORE_VERSION.to_le_bytes());
    payload.extend_from_slice(&(bank.selected as u32).to_le_bytes());
    payload.extend_from_slice(&(bank.presets.len() as u32).to_le_bytes());
    for (index, preset) in bank.presets.iter().enumerate() {
        encode_preset(&mut payload, preset, index);
    }
    payload
}

fn encode_preset(payload: &mut Vec<u8>, preset: &PumpPreset, index: usize) {
    let name = sanitize_preset_name(&preset.name, index);
    payload.extend_from_slice(&(name.len() as u32).to_le_bytes());
    payload.extend_from_slice(name.as_bytes());
    payload.extend_from_slice(&preset.mix.to_le_bytes());
    payload.extend_from_slice(&preset.depth.to_le_bytes());
    payload.extend_from_slice(&preset.phase_offset.to_le_bytes());
    payload.extend_from_slice(&preset.output_gain_db.to_le_bytes());
    payload.extend_from_slice(&(preset.sync_division as u32).to_le_bytes());
    encode_curve(payload, &preset.editable_curve);
    payload.extend_from_slice(&(preset.quick_slots.len() as u32).to_le_bytes());
    for slot in &preset.quick_slots {
        encode_curve(payload, &slot.curve);
    }
}

fn encode_curve(payload: &mut Vec<u8>, curve: &EditableCurve) {
    let normalized_curve = curve.clone().normalized();
    let node_count = normalized_curve.nodes.len().clamp(2, MAX_EDITABLE_NODES);
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
}

fn decode_preset_bank_payload(payload: &[u8]) -> Result<PumpPresetBank, String> {
    let mut cursor = Cursor::new(payload);
    let magic = read_u32(&mut cursor).ok_or_else(|| "invalid preset store header".to_string())?;
    if magic != u32::from_le_bytes(*PRESET_STORE_MAGIC) {
        return Err("unknown preset store format".to_string());
    }
    let version =
        read_u32(&mut cursor).ok_or_else(|| "invalid preset store version".to_string())?;
    if !(1..=PRESET_STORE_VERSION).contains(&version) {
        return Err(format!("unsupported preset store version `{version}`"));
    }
    let selected = read_u32(&mut cursor)
        .map(|value| value as usize)
        .ok_or_else(|| "invalid preset selected index".to_string())?;
    let count = read_u32(&mut cursor)
        .map(|value| value as usize)
        .ok_or_else(|| "invalid preset count".to_string())?;
    if count == 0 || count > MAX_PRESETS {
        return Err("invalid preset count bounds".to_string());
    }
    let mut presets = Vec::with_capacity(count);
    for index in 0..count {
        let name_len = read_u32(&mut cursor)
            .map(|value| value as usize)
            .ok_or_else(|| "invalid preset name length".to_string())?;
        if name_len == 0 || name_len > 256 {
            return Err("invalid preset name length bounds".to_string());
        }
        let mut name_bytes = vec![0_u8; name_len];
        std::io::Read::read_exact(&mut cursor, &mut name_bytes)
            .map_err(|_| "invalid preset name".to_string())?;
        let raw_name =
            std::str::from_utf8(&name_bytes).map_err(|_| "invalid preset name utf8".to_string())?;
        let mix = read_f32(&mut cursor).ok_or_else(|| "invalid preset mix".to_string())?;
        let depth = read_f32(&mut cursor).ok_or_else(|| "invalid preset depth".to_string())?;
        let phase_offset =
            read_f32(&mut cursor).ok_or_else(|| "invalid preset phase offset".to_string())?;
        let output_gain_db =
            read_f32(&mut cursor).ok_or_else(|| "invalid preset output gain".to_string())?;
        let sync_division = read_u32(&mut cursor)
            .map(|value| value as usize)
            .ok_or_else(|| "invalid preset sync division".to_string())?;
        let node_count = read_u32(&mut cursor)
            .map(|value| value as usize)
            .ok_or_else(|| "invalid preset node count".to_string())?;
        let editable_curve = decode_curve(&mut cursor, node_count)?;
        let quick_slots = if version >= 2 {
            decode_quick_slots(&mut cursor)?
        } else {
            seeded_quick_shape_slots()
        };
        presets.push(PumpPreset {
            name: sanitize_preset_name(raw_name, index),
            is_read_only: false,
            mix,
            depth,
            phase_offset,
            output_gain_db,
            sync_division: sync_division.min(MAX_SYNC_DIVISION as usize),
            editable_curve,
            quick_slots,
        });
    }
    if cursor.position() != payload.len() as u64 {
        return Err("unexpected trailing preset store bytes".to_string());
    }
    Ok(PumpPresetBank {
        selected: selected.min(count.saturating_sub(1)),
        presets,
    })
}

fn decode_curve(cursor: &mut Cursor<&[u8]>, node_count: usize) -> Result<EditableCurve, String> {
    if !(2..=MAX_EDITABLE_NODES).contains(&node_count) {
        return Err("invalid node count bounds".to_string());
    }
    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let x = read_f32(cursor).ok_or_else(|| "invalid curve node x".to_string())?;
        let y = read_f32(cursor).ok_or_else(|| "invalid curve node y".to_string())?;
        nodes.push(CurveNode { x, y });
    }
    let mut segments = Vec::with_capacity(node_count.saturating_sub(1));
    for _ in 0..node_count.saturating_sub(1) {
        let tension = read_f32(cursor).ok_or_else(|| "invalid curve segment".to_string())?;
        segments.push(CurveSegment { tension });
    }
    Ok(EditableCurve { nodes, segments }.normalized())
}

fn decode_quick_slots(cursor: &mut Cursor<&[u8]>) -> Result<Vec<QuickShapeSlot>, String> {
    let count = read_u32(cursor)
        .map(|value| value as usize)
        .ok_or_else(|| "invalid preset quick slot count".to_string())?;
    let mut slots = Vec::with_capacity(count);
    for _ in 0..count {
        let node_count = read_u32(cursor)
            .map(|value| value as usize)
            .ok_or_else(|| "invalid preset quick slot node count".to_string())?;
        let curve = decode_curve(cursor, node_count)?;
        slots.push(QuickShapeSlot { curve });
    }
    Ok(slots)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "pump-preset-store-{label}-{}-{stamp}.bin",
            std::process::id()
        ))
    }

    #[test]
    fn preset_store_roundtrip_preserves_bank() {
        let path = temp_path("roundtrip");
        let bank = PumpPresetBank {
            selected: 1,
            presets: vec![
                PumpPreset {
                    name: "Init".to_string(),
                    is_read_only: false,
                    mix: 1.0,
                    depth: 0.7,
                    phase_offset: 0.0,
                    output_gain_db: 0.0,
                    sync_division: 4,
                    editable_curve: default_editable_curve(),
                    quick_slots: seeded_quick_shape_slots(),
                },
                PumpPreset {
                    name: "Verse".to_string(),
                    is_read_only: false,
                    mix: 0.25,
                    depth: 0.55,
                    phase_offset: 0.13,
                    output_gain_db: -1.5,
                    sync_division: 3,
                    editable_curve: EditableCurve {
                        nodes: vec![
                            CurveNode { x: 0.0, y: 1.0 },
                            CurveNode { x: 0.42, y: 0.2 },
                            CurveNode { x: 1.0, y: 0.9 },
                        ],
                        segments: vec![
                            CurveSegment { tension: -0.4 },
                            CurveSegment { tension: 0.2 },
                        ],
                    }
                    .normalized(),
                    quick_slots: seeded_quick_shape_slots(),
                },
            ],
        };

        save_preset_bank_to_path(&path, &bank).expect("preset store save should succeed");
        let payload = fs::read(&path).expect("preset store file should exist");
        let loaded = decode_preset_bank_payload(&payload).expect("preset store decode should pass");
        assert_eq!(loaded, bank);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn preset_store_decode_rejects_invalid_magic() {
        let mut payload = vec![0_u8; 16];
        payload[..4].copy_from_slice(b"NOPE");
        let error =
            decode_preset_bank_payload(&payload).expect_err("invalid magic should be rejected");
        assert_eq!(error, "unknown preset store format");
    }

    #[test]
    fn preset_store_v1_decode_seeds_quick_slots() {
        let mut payload = Vec::new();
        payload.extend_from_slice(PRESET_STORE_MAGIC);
        payload.extend_from_slice(&1_u32.to_le_bytes());
        payload.extend_from_slice(&0_u32.to_le_bytes());
        payload.extend_from_slice(&1_u32.to_le_bytes());
        payload.extend_from_slice(&4_u32.to_le_bytes());
        payload.extend_from_slice(b"Init");
        payload.extend_from_slice(&1.0_f32.to_le_bytes());
        payload.extend_from_slice(&1.0_f32.to_le_bytes());
        payload.extend_from_slice(&0.0_f32.to_le_bytes());
        payload.extend_from_slice(&0.0_f32.to_le_bytes());
        payload.extend_from_slice(&4_u32.to_le_bytes());
        payload.extend_from_slice(&3_u32.to_le_bytes());
        payload.extend_from_slice(&0.0_f32.to_le_bytes());
        payload.extend_from_slice(&1.0_f32.to_le_bytes());
        payload.extend_from_slice(&0.5_f32.to_le_bytes());
        payload.extend_from_slice(&0.2_f32.to_le_bytes());
        payload.extend_from_slice(&1.0_f32.to_le_bytes());
        payload.extend_from_slice(&1.0_f32.to_le_bytes());
        payload.extend_from_slice(&(-0.3_f32).to_le_bytes());
        payload.extend_from_slice(&(0.2_f32).to_le_bytes());

        let decoded =
            decode_preset_bank_payload(&payload).expect("v1 preset store payload should decode");
        assert_eq!(decoded.presets[0].quick_slots, seeded_quick_shape_slots());
    }
}
