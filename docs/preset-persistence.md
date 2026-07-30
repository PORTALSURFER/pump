# Preset persistence contract

## A/B sound-state contract

Pump stores two complete editable sound snapshots. Side A is the compatibility
default for v2-v13 project payloads; those payloads migrate by cloning the
legacy active state into both sides and selecting A. v14 and v15 payloads append
the active side and both working snapshots. v16 appends one durable
stored/reference snapshot for each side; older payloads seed those references
from their working snapshots and therefore reopen clean.

Changing A/B publishes the selected snapshot through the existing atomic audio
parameter boundary. The processor never reads the UI-owned snapshot lock.
Copy is explicit and directional (active to inactive), leaves the active audio
unchanged, and is available by Command-clicking the center switch or
Option-clicking the inactive side button. Working edits are captured
automatically as they occur; there is no manual Command-click store step or
`Stored`/`Modified` workflow in the editor. Preset selection and quick-slot edits
apply only to the active side; saving a preset captures that side's complete
editable state. Host automation for the appended `Sound` parameter switches a
pre-published realtime selector in the callback; the UI projection only
consumes the repaint cue, avoiding a realtime lock while keeping the host-facing
A/B value authoritative with the editor closed.

Pump treats preset-bank mutations as durable transactions. Create, overwrite,
rename, full-bank replacement, legacy preset quick-slot updates, and selected
index changes are staged in a cloned bank. Pump writes that candidate bank with
the atomic temporary-file replacement path before changing runtime state.

## Host bypass is project state, not preset state

The host-visible `Bypass` parameter is excluded from `PumpPreset`, the
persisted preset bank, preset dirty comparisons, editor undo/redo history, and
every preset load/save operation. Loading or saving a Pump preset therefore
never changes whether the plug-in is active or bypassed.

Bypass is stored only in the host project-state payload (v12 and newer).
Project states from v2 through v11 migrate to `ACTIVE`, keeping host bypass
automation and session restoration authoritative during preset auditioning.

If directory creation, temporary-file writing, or final rename fails, Pump:

- returns `PresetMutationError::PersistenceFailed` to the caller;
- leaves the in-memory preset bank unchanged;
- leaves the on-disk bank unchanged, so relaunch restores the last durable state;
- leaves active parameters unchanged when selecting a preset fails; and
- keeps a visible `NOT SAVED - CHECK PRESET FOLDER` warning in both GUI renderers
  until a later preset-bank write succeeds.

Validation, capacity, invalid-index, and state-access failures have distinct
`PresetMutationError` variants and do not claim durable success. Pump does not
currently expose a preset-delete mutation; if one is added, it must use the same
stage, persist, then commit contract.

Filesystem logging is supplemental diagnostic detail. User-visible correctness
comes from the structured API result, rollback behavior, and persistent GUI
warning.
