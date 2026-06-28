use super::*;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(test)]
use std::{cell::RefCell, panic::AssertUnwindSafe};

const CURVE_SLOT_STORE_MAGIC: &[u8; 4] = b"PCSL";
const CURVE_SLOT_STORE_VERSION: u32 = 1;
const CURVE_SLOT_STORE_PATH_ENV: &str = "PUMP_GLOBAL_CURVE_SLOTS_PATH";
const CURVE_SLOT_STORE_FILE_NAME: &str = "curve-slots.bin";

static CURVE_SLOT_STORE_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
thread_local! {
    static TEST_CURVE_SLOT_PATH_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Run one closure with a thread-local global-slot path override enabled.
#[cfg(test)]
pub(crate) fn with_test_curve_slot_path<R>(path: PathBuf, f: impl FnOnce() -> R) -> R {
    TEST_CURVE_SLOT_PATH_OVERRIDE.with(|slot| {
        let previous = slot.replace(Some(path));
        let result = std::panic::catch_unwind(AssertUnwindSafe(f));
        slot.replace(previous);
        match result {
            Ok(value) => value,
            Err(error) => std::panic::resume_unwind(error),
        }
    })
}

/// Load the globally persisted curve slots.
pub(crate) fn load_global_curve_slots() -> Result<Vec<GlobalCurveSlot>, String> {
    let _guard = CURVE_SLOT_STORE_LOCK
        .lock()
        .map_err(|_| "curve slot store lock poisoned".to_string())?;
    load_global_curve_slots_unlocked()
}

/// Store the provided curve into a global slot and persist the full slot bank.
pub(crate) fn store_global_curve_slot(index: usize, curve: &EditableCurve) -> Result<(), String> {
    if index >= GLOBAL_CURVE_SLOT_COUNT {
        return Err("global curve slot index out of bounds".to_string());
    }
    let _guard = CURVE_SLOT_STORE_LOCK
        .lock()
        .map_err(|_| "curve slot store lock poisoned".to_string())?;
    let mut slots = load_global_curve_slots_unlocked()?;
    slots[index].curve = Some(curve.clone().normalized());
    persist_global_curve_slots_unlocked(&slots)
}

fn load_global_curve_slots_unlocked() -> Result<Vec<GlobalCurveSlot>, String> {
    let Some(path) = persistence_file_path() else {
        return Ok(empty_global_curve_slots());
    };
    if !path.is_file() {
        return Ok(empty_global_curve_slots());
    }
    let payload = fs::read(&path).map_err(|error| {
        format!(
            "failed to read curve slot store `{}`: {error}",
            path.display()
        )
    })?;
    decode_global_curve_slot_payload(&payload)
}

fn persist_global_curve_slots_unlocked(slots: &[GlobalCurveSlot]) -> Result<(), String> {
    let Some(path) = persistence_file_path() else {
        return Ok(());
    };
    save_global_curve_slots_to_path(&path, slots)
}

fn empty_global_curve_slots() -> Vec<GlobalCurveSlot> {
    vec![GlobalCurveSlot { curve: None }; GLOBAL_CURVE_SLOT_COUNT]
}

fn normalize_global_curve_slots(mut slots: Vec<GlobalCurveSlot>) -> Vec<GlobalCurveSlot> {
    if slots.len() > GLOBAL_CURVE_SLOT_COUNT {
        slots.truncate(GLOBAL_CURVE_SLOT_COUNT);
    }
    while slots.len() < GLOBAL_CURVE_SLOT_COUNT {
        slots.push(GlobalCurveSlot { curve: None });
    }
    for slot in &mut slots {
        if let Some(curve) = slot.curve.as_mut() {
            *curve = curve.clone().normalized();
        }
    }
    slots
}

fn persistence_file_path() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_CURVE_SLOT_PATH_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return Some(path);
    }
    if cfg!(test) && std::env::var_os(CURVE_SLOT_STORE_PATH_ENV).is_none() {
        return None;
    }
    if let Some(path) =
        std::env::var_os(CURVE_SLOT_STORE_PATH_ENV).filter(|value| !value.is_empty())
    {
        return Some(PathBuf::from(path));
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(base) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
            return Some(
                PathBuf::from(base)
                    .join("PORTALSURFER")
                    .join("Pump")
                    .join(CURVE_SLOT_STORE_FILE_NAME),
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
                    .join(CURVE_SLOT_STORE_FILE_NAME),
            );
        }
        if let Some(base) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
            return Some(
                PathBuf::from(base)
                    .join("pump")
                    .join(CURVE_SLOT_STORE_FILE_NAME),
            );
        }
        std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(|base| {
                PathBuf::from(base)
                    .join(".config")
                    .join("pump")
                    .join(CURVE_SLOT_STORE_FILE_NAME)
            })
    }
}

fn save_global_curve_slots_to_path(path: &Path, slots: &[GlobalCurveSlot]) -> Result<(), String> {
    let payload = encode_global_curve_slot_payload(slots);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create curve slot store directory `{}`: {error}",
                parent.display()
            )
        })?;
    }
    let temp_path = temporary_store_path(path);
    fs::write(&temp_path, payload).map_err(|error| {
        format!(
            "failed to write temporary curve slot store `{}`: {error}",
            temp_path.display()
        )
    })?;
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(path);
        if let Err(retry_error) = fs::rename(&temp_path, path) {
            let _ = fs::remove_file(&temp_path);
            return Err(format!(
                "failed to finalize curve slot store `{}` after `{}`: {retry_error}",
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
        .unwrap_or_else(|| CURVE_SLOT_STORE_FILE_NAME.to_string());
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    path.with_file_name(format!("{file_name}.tmp-{pid}-{stamp}"))
}

fn encode_global_curve_slot_payload(slots: &[GlobalCurveSlot]) -> Vec<u8> {
    let normalized_slots = normalize_global_curve_slots(slots.to_vec());
    let mut payload = Vec::new();
    payload.extend_from_slice(CURVE_SLOT_STORE_MAGIC);
    payload.extend_from_slice(&CURVE_SLOT_STORE_VERSION.to_le_bytes());
    payload.extend_from_slice(&(GLOBAL_CURVE_SLOT_COUNT as u32).to_le_bytes());
    for slot in normalized_slots {
        match slot.curve {
            Some(curve) => {
                payload.push(1);
                encode_curve(&mut payload, &curve);
            }
            None => payload.push(0),
        }
    }
    payload
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

fn decode_global_curve_slot_payload(payload: &[u8]) -> Result<Vec<GlobalCurveSlot>, String> {
    let mut cursor = Cursor::new(payload);
    let magic =
        read_u32(&mut cursor).ok_or_else(|| "invalid curve slot store header".to_string())?;
    if magic != u32::from_le_bytes(*CURVE_SLOT_STORE_MAGIC) {
        return Err("unknown curve slot store format".to_string());
    }
    let version =
        read_u32(&mut cursor).ok_or_else(|| "invalid curve slot store version".to_string())?;
    if version != CURVE_SLOT_STORE_VERSION {
        return Err(format!("unsupported curve slot store version `{version}`"));
    }
    let count = read_u32(&mut cursor)
        .map(|value| value as usize)
        .ok_or_else(|| "invalid curve slot count".to_string())?;
    if count > GLOBAL_CURVE_SLOT_COUNT {
        return Err("invalid curve slot count bounds".to_string());
    }
    let mut slots = Vec::with_capacity(count);
    for _ in 0..count {
        let occupied = read_u8(&mut cursor).ok_or_else(|| "invalid curve slot flag".to_string())?;
        let curve = match occupied {
            0 => None,
            1 => {
                let node_count = read_u32(&mut cursor)
                    .map(|value| value as usize)
                    .ok_or_else(|| "invalid curve slot node count".to_string())?;
                Some(decode_curve(&mut cursor, node_count)?)
            }
            _ => return Err("invalid curve slot occupancy flag".to_string()),
        };
        slots.push(GlobalCurveSlot { curve });
    }
    if cursor.position() != payload.len() as u64 {
        return Err("unexpected trailing curve slot store bytes".to_string());
    }
    Ok(normalize_global_curve_slots(slots))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "pump-global-curve-slots-{label}-{}-{stamp}.bin",
            std::process::id()
        ))
    }

    #[test]
    fn curve_slot_store_roundtrip_preserves_empty_and_occupied_slots() {
        let path = temp_path("roundtrip");
        let mut slots = empty_global_curve_slots();
        slots[2].curve = Some(default_editable_curve());

        save_global_curve_slots_to_path(&path, &slots).expect("curve slot save should succeed");
        let payload = fs::read(&path).expect("curve slot store file should exist");
        let loaded =
            decode_global_curve_slot_payload(&payload).expect("curve slot decode should pass");
        assert_eq!(loaded, slots);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn store_global_curve_slot_updates_only_target_slot() {
        let path = temp_path("store-one");
        with_test_curve_slot_path(path.clone(), || {
            let mut curve = default_editable_curve();
            curve.nodes[1].y = 0.25;
            store_global_curve_slot(3, &curve).expect("global slot store should succeed");
            let slots = load_global_curve_slots().expect("global slots should load");

            assert_eq!(slots.len(), GLOBAL_CURVE_SLOT_COUNT);
            assert!(slots
                .iter()
                .enumerate()
                .all(|(index, slot)| index == 3 || slot.curve.is_none()));
            assert_eq!(slots[3].curve, Some(curve.normalized()));
        });
        let _ = fs::remove_file(path);
    }

    #[test]
    fn curve_slot_decode_rejects_invalid_magic() {
        let mut payload = vec![0_u8; 16];
        payload[..4].copy_from_slice(b"NOPE");
        let error = decode_global_curve_slot_payload(&payload)
            .expect_err("invalid magic should be rejected");
        assert_eq!(error, "unknown curve slot store format");
    }
}
