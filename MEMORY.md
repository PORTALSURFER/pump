# Memory

- Last Updated (UTC): 2026-06-26 12:42:15 UTC
- Active Mission: Update Pump's GUI dependency surface to track the latest pinned Radiant revision.
- Current Workstream: Radiant is pinned at `119f95cfebab84687b7af870f3bf6e385f365346`, Pump has a GUI embedded-surface smoke test for that API, and the local CI harness handles the default no-feature lane on macOS Bash.

## Current State

- `AGENTS.md` is a portal that points to current-state and plan files.
- Active execution order lives in `docs/plans/active/todo.md`.
- Detailed plan navigation lives in `docs/plans/index.md`.
- Local preflight entrypoint is `scripts/run_agent_request.sh`.
- Local changelog generator is `scripts/update_changelog.sh`.
- Push-time changelog updater is `.github/workflows/changelog.yml`.
- Shared monotonic timing utility is `src/time_utils.rs`.
- The Pump curve editor now exposes an editor-local `Snap` checkbox and `Auto`/override grid dropdown in place of the old reset row.
- Effective grid lines are rendered brighter for the selected musical division while the faint background micro-grid remains visible.
- Curve point insertion, dragging, and segment translation now snap to the active vertical beat guides plus quarter-step horizontal bands when snap is effectively enabled.
- Holding `s` temporarily inverts snapping, while preset save moved to `Shift+S`.
- Undo and redo are now bound to `Ctrl+Z` and `Ctrl+Y` instead of the older `u` / `Shift+u` shortcuts.
- Pump now depends on the latest `PORTALSURFER/radiant` main revision with a full `rev` pin, and `gui::tests::radiant_embedded_gui_surface_renders_at_pump_design_size` verifies Radiant can emit a frame for Pump-sized GUI content.
- `scripts/ci.sh` now avoids the macOS Bash `set -u` empty-array failure when no feature flags are requested.

## Immediate Next Action

- Run changelog/local verification, then commit and push the Pump branch before updating the audiodev superproject pointer.
