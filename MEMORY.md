# Memory

- Last Updated (UTC): 2026-03-29 09:49:04 UTC
- Active Mission: Keep Pump's editor UX moving forward without expanding preset/state scope unnecessarily.
- Current Workstream: Beat-grid emphasis, snap controls, temporary snap inversion, and grid override UI are implemented, and the snap control now renders as a lit/unlit checkbox square instead of a switch-style toggle.

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

## Immediate Next Action

- No active Pump-specific task is currently running; continue from `docs/plans/active/todo.md` item `1` when the next request arrives, keeping Pump on the pinned `toybox` revision `ec53c316c6212e474db3ae81c269b5b5c9fcf177`.
