# Memory

- Last Updated (UTC): 2026-07-11 12:10:00 UTC
- Active Mission: Adopt Radiant's native sampled curve-area fill in Pump.
- Current Workstream: PR #12 on branch `codex/pump-radiant-curve-area-fill` is ready and waiting for user review; it pins merged Radiant PR #1407 and replaces the Radiant/VST3 editor's 96 opaque fill rectangles with one bottom-baselined gradient `FillPath`.

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
- `Cargo.toml` pins Radiant to merged curve-area-fill commit `78b16cfe5369304420cd9345ee689796c00585e6` and Toybox to merged main commit `2c0317e6cc777258d6bf573cf8e802fd96dc02fa`.
- The Radiant/VST3 attenuation fill uses `push_sampled_curve_area_fill` with one 96-interval path and a top-to-bottom alpha fade; the Toybox/CLAP fill remains unchanged.
- Radiant curve endpoints remain protected wrapped-Y anchors; only interior points can be drag-through removed.
- Focused Radiant coverage includes sticky boundary resistance, single- and multi-point crossing, reverse-drag restoration, release commit, and endpoint anchoring.
- `cargo fmt --all -- --check`, `cargo test radiant_editor -- --nocapture`, `bash scripts/ci.sh`, `bash scripts/ci_local.sh`, and `VST3_SDK_DIR=/Users/portalsurfer/lib/vst3sdk bash scripts/ci.sh --vst3` are green on the new Radiant pin.
- The release VST3 build passes bundle signing verification; Bitwig PID `87704` still maps the previous Pump binary and must restart or fully unload before visual testing.

## Immediate Next Action

- Wait for explicit user review and sign-off on Pump PR #12 before merge.
