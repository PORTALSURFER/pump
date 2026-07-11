# Memory

- Last Updated (UTC): 2026-07-11 12:58:13 UTC
- Active Mission: Render Pump's declarative GUI entirely through Radiant while Toybox owns reusable plugin-host infrastructure.
- Current Workstream: PR #12 on branch `codex/pump-radiant-curve-area-fill` is back in implementation after hosted testing exposed Pump's obsolete Cocoa primitive renderer.

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
- `Cargo.toml` pins Radiant to embedded Vello commit `9b45df71893a71f165fcae4183a189d664cecb10` and Toybox to hosted-view commit `805fef15cb6021b35120a34f6008d106f6814065`.
- Toybox now initializes the declarative editor's logical size before its first hosted paint.
- Toybox forwards key events that Radiant does not handle through AppKit's responder chain.
- Toybox preserves the last host-provided logical size while closing and reopening its native view.
- Toybox preserves Option-generated text and leaves Command-modified shortcuts to AppKit's host responder chain.
- Pump's macOS VST3 adapter now supplies `RadiantPumpEditor` to Toybox's generic `RadiantVst3HostedGui`; Pump's Cocoa/AppKit view and primitive paint replay have been deleted.
- The Radiant/VST3 attenuation fill uses `push_sampled_curve_area_fill` with one 96-interval path and a top-to-bottom alpha fade; the Toybox/CLAP fill remains unchanged.
- Radiant curve endpoints remain protected wrapped-Y anchors; only interior points can be drag-through removed.
- Focused Radiant coverage includes sticky boundary resistance, single- and multi-point crossing, reverse-drag restoration, release commit, and endpoint anchoring.
- Full manual VST3 validation passes: format, feature-specific clippy, and all 190 tests. Toybox's main-thread smoke host also attaches and draws a gradient `FillPath` through embedded Vello.
- A fresh signed artifact is installed at `dist/pump-v0.2.0-macos.vst3`; its binary SHA-256 is `9fa7f8981ab5d5c7e410a68cbedf0c6e66d63424b44106f97ec7c66a7be46738`.
- Bitwig plugin-host PID `56167` still maps the previous binary and must unload or restart before testing this rebuild.
- `scripts/run_agent_request.sh` is currently blocked by the root screenshot-coverage policy not listing Pump, which predates this GUI migration.

## Immediate Next Action

- Complete Pump validation, build a fresh release VST3, and verify the Radiant/Vello surface and attenuation fill in a real host before returning PR #12 to user review.
