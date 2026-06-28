# Memory

- Last Updated (UTC): 2026-06-28 17:06:07 UTC
- Active Mission: Add OPT-926 global Pump curve slot swatches with load/store and deviation state.
- Current Workstream: Branch `wsvasek/opt-926-pump-add-global-curve-slot-swatches-with-loadstore-and` adds 8 globally persisted curve slots shared across Pump instances, with click-to-load, Cmd-click-to-store, empty-slot no-op behavior, miniature previews, and red loaded-slot deviation feedback.

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
- `bash scripts/ci.sh` and `VST3_SDK_DIR=/Users/portalsurfer/lib/vst3sdk cargo test --features vst3 cocoa_gui -- --nocapture` are green.

## Immediate Next Action

- Regenerate the Pump lockfile for the merged Toybox rev, validate Pump, commit/push, then merge the Pump PR and sync the audiodev superproject pointer.
