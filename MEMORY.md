# Memory

- Last Updated (UTC): 2026-06-28 12:32:17 UTC
- Active Mission: Pick up the Toybox/Radiant Ioskeley Mono default font in Pump.
- Current Workstream: Branch `codex/pump-ioskeley-toybox-rev` updates Pump's pinned `toybox` dependency to merged Toybox main commit `1000011b94a73d57aa41f6f988d821c4de7cf862`, where Patchbay/Radiant vector text now prefers bundled Ioskeley Mono before the previous Sometype Mono fallback.

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
- The VST3 AppKit tests now prove the hosted `PumpRadiantEditorView` contains a live Radiant runtime with visible `PUMP` text, fill, and curve polyline paint primitives after `IPlugView::attached`.
- The Radiant curve widget now previews a new node while hovering sampled curve segments, inserts on segment click or blank-canvas click, hands the inserted point to the existing active-node drag/release path, paints an Option-held segment hover highlight, suppresses insert preview during Option-line hover, and adjusts segment curvature on Option-drag.
- The macOS VST3 AppKit editor view now installs mouse tracking and forwards hover/modifier events so Option-hover can work in hosts.
- The Radiant curve widget now paints a copper/warning/mint playback-position marker at `GuiStatus::phase()` whenever host beat timeline data is available or playback is active.
- The Radiant editor now refreshes/reprojects its declarative surface before realtime paint-plan generation, and a VST3-gated regression test verifies the playhead marker moves after `GuiStatus` changes without dispatching a pointer event.
- The macOS VST3 AppKit editor view now installs a 30 Hz redraw driver thread, retains the view while active, posts `playheadRedrawTick:` to the AppKit main thread, wakes the main run loop, forces `displayIfNeeded` when the playhead should animate, and stops/joins/releases on close/dealloc so the Radiant playhead marker can move in realtime without relying on pointer events or host run-loop timers.
- GitHub Actions CI now sets `CARGO_NET_GIT_FETCH_WITH_CLI=true` and configures git to use the `RADIANT_REPO_TOKEN` repository secret so Cargo can fetch the private pinned Radiant dependency.
- The Windows dropdown screenshot regression test now creates `MAX_PRESETS - 1` additional presets instead of exceeding the model cap.
- The Windows dropdown regression now uses Pump's headless declarative renderer plus reducer checks for popup-over-curve geometry and preset/division selection behavior, avoiding the GitHub Windows runner's native WGPU readback access violation.
- Pump now depends on a Toybox revision with Ableton-compatible macOS VST3 bundle entry symbols.
- Pump now depends on the Toybox revision that makes Ioskeley Mono the default bundled Radiant/Patchbay vector text font.
- `scripts/ci.sh` now avoids the macOS Bash `set -u` empty-array failure when no feature flags are requested.

## Immediate Next Action

- Run changelog/local CI, commit and push the Pump branch, then update the audiodev superproject pointers for `toybox` and `pump`. Latest validation passed: `cargo fmt --all -- --check`, `cargo test radiant_editor -- --nocapture`, `bash scripts/ci.sh`, `cargo check`, `VST3_SDK_DIR=/Users/portalsurfer/lib/vst3sdk cargo test --features vst3 cocoa_gui -- --nocapture`, and `VST3_SDK_DIR=/Users/portalsurfer/lib/vst3sdk bash scripts/ci.sh --vst3`.
