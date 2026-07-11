# Memory

- Last Updated (UTC): 2026-07-11 09:53:59 UTC
- Active Mission: Add a subtle fill beneath Pump's curve to visualize attenuation.
- Current Workstream: Branch `codex/pump-attenuation-fill` adds matching low-alpha curve fills to the Toybox and Radiant editor paths, with focused paint-plan coverage.

## Current State

- `AGENTS.md` is a portal that points to current-state and plan files.
- Active execution order lives in `docs/plans/active/todo.md`.
- Detailed plan navigation lives in `docs/plans/index.md`.
- Local preflight entrypoint is `scripts/run_agent_request.sh`.
- Local changelog generator is `scripts/update_changelog.sh`.
- Push-time changelog updater is `.github/workflows/changelog.yml`.
- Shared monotonic timing utility is `src/time_utils.rs`.
- `src/params/global_curve_slots.rs` owns the new global slot store (`curve-slots.bin`) with atomic temp-file replacement, thread-local test path overrides, empty-slot support, and binary serialization tests.
- `PumpParams` exposes global slot snapshot/load/store/deviation helpers; preset-bank quick-slot payloads remain for backwards compatibility but are no longer the active UI slot source.
- The Toybox UI slot strip now reads global slots, uses Cmd-store via the new Toybox region `command_down` modifier, treats empty normal-clicks as no-ops, paints loaded-slot deviation in red, and keeps all visible slot swatches the same size.
- The Radiant/VST3 editor now has its own compact 8-slot row with the same load/store/deviation behavior and uniform fixed-size slot swatches.
- `Cargo.toml` pins Toybox to merged main commit `2c0317e6cc777258d6bf573cf8e802fd96dc02fa`, which adds command modifier plumbing to region actions.
- Radiant curve endpoints remain protected wrapped-Y anchors; only interior points can be drag-through removed.
- Focused Radiant coverage includes sticky boundary resistance, single- and multi-point crossing, reverse-drag restoration, release commit, and endpoint anchoring.
- `cargo test radiant_editor -- --nocapture`, `VST3_SDK_DIR=/Users/portalsurfer/lib/vst3sdk cargo test --features vst3 cocoa_gui -- --nocapture`, `bash scripts/ci.sh`, and `VST3_SDK_DIR=/Users/portalsurfer/lib/vst3sdk bash scripts/ci.sh --vst3` are green.

## Immediate Next Action

- Push the completed branch and open its ready-for-review PR. The fresh review bundle is `/Users/portalsurfer/dev/audiodev/dist/pump-v0.2.0-macos.vst3` with binary SHA-256 `94dc02ad79f5115c8e37ffd903e72179b5f89b18455431e58798ee608b3f3a83`; restart or fully unload Bitwig's existing Pump host before testing it.
