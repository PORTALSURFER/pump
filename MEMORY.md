# Memory

- Last Updated (UTC): 2026-06-27 07:46:49 UTC
- Active Mission: Fix the signed-off Pump Radiant PR's GitHub Actions dependency-fetch failure, then merge it.
- Current Workstream: Pump pins `PORTALSURFER/radiant` main at `119f95cfebab84687b7af870f3bf6e385f365346`, uses Toybox `593b67a91d25ee22668047714a54e9f521d125e1`, and `dist/pump-v0.2.0-macos.vst3` exports Ableton-required `_bundleEntry`, `_bundleExit`, and `_GetPluginFactory`.

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
- GitHub Actions CI now sets `CARGO_NET_GIT_FETCH_WITH_CLI=true` and configures git to use the `RADIANT_REPO_TOKEN` repository secret so Cargo can fetch the private pinned Radiant dependency.
- Pump now depends on a Toybox revision with Ableton-compatible macOS VST3 bundle entry symbols.
- `scripts/ci.sh` now avoids the macOS Bash `set -u` empty-array failure when no feature flags are requested.

## Immediate Next Action

- Commit and push the explicit `RADIANT_REPO_TOKEN` CI credential fix, wait for GitHub Actions to pass, then merge the signed-off Pump PR before updating/merging the audiodev superproject PR. Local Pump CI is green; the rebuilt root `dist` Pump VST3 binary SHA-256 from the latest code-bearing commit is `339085d94859d7428eb65ff9fa6e35fd79a0023e96c38447a112fa12dbf28d71`.
