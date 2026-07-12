# Memory

- Last Updated (UTC): 2026-07-12 08:59:09 UTC
- Active Mission: Make Pump's curve playhead visually distinct from editable curve nodes for OPT-935.
- Current Workstream: Pump PR #13 on branch `wsvasek/opt-935-pump-make-curve-playhead-dot-visually-distinct-from-curve` is signed off by the user and addressing one P2 review item before merge: node primitives must paint before the playhead so exact phase/node overlaps preserve the magenta indicator.

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
- `Cargo.toml` pins Radiant to embedded animation-clock commit `6575b0c9a6b5abad17f711a36b832b7e7434e7b1` and Toybox to hosted-view commit `a3e0279619bcd087bdcc803c6fc4ca6a65ade33b`.
- Radiant acquires and recovers the presentation surface before rendering, preventing a Lost/Outdated recovery frame from blitting an unrendered replacement target.
- Radiant embedded validation shares the scene encoder's clip state, so unsupported surfaces inside suppressed clips are ignored consistently.
- Toybox hosted views can own and forward `NativeTextOptions` for portable embedded fonts without plugin-local rendering code.
- Radiant trait-based embedded renders advance monotonic elapsed time, keeping focused text-input caret animation live through Toybox's normal renderer call.
- Toybox now initializes the declarative editor's logical size before its first hosted paint.
- Toybox forwards key events that Radiant does not handle through AppKit's responder chain.
- Toybox preserves the last host-provided logical size while closing and reopening its native view.
- Toybox preserves Option-generated text and leaves Command-modified shortcuts to AppKit's host responder chain.
- Pump's macOS VST3 adapter now supplies `RadiantPumpEditor` to Toybox's generic `RadiantVst3HostedGui`; Pump's Cocoa/AppKit view and primitive paint replay have been deleted.
- The Radiant/VST3 attenuation fill uses `push_sampled_curve_area_fill` with one 96-interval path and a top-to-bottom alpha fade; the Toybox/CLAP fill remains unchanged.
- Radiant curve endpoints remain protected wrapped-Y anchors; only interior points can be drag-through removed.
- Focused Radiant coverage includes sticky boundary resistance, single- and multi-point crossing, reverse-drag restoration, release commit, and endpoint anchoring.
- Full manual VST3 validation passes: format, feature-specific clippy, and all 190 tests. Toybox's main-thread smoke host also attaches and draws a gradient `FillPath` through embedded Vello.
- A fresh signed artifact is installed at `dist/pump-v0.2.0-macos.vst3`; its binary SHA-256 is `fd6d7bd12e9b34a586fa7b77212b1ce37481206d601355e46df29412a4c0b6e5`.
- Bitwig plugin-host PID `56167` still maps the previous binary and must unload or restart before testing this rebuild.
- `scripts/run_agent_request.sh` is currently blocked by the root screenshot-coverage policy not listing Pump, which predates this GUI migration.
- OPT-935 now reserves magenta for the playhead core/ring/glow in both Pump curve renderers instead of reusing normal, hovered, selected, or preview node colors. All 180 tests, VST3 clippy with warnings denied, focused VST3 playhead tests, and headless playback screenshots at 315x211, 420x282, 525x352, and 630x423 pass.
- The signed review artifact is `dist/pump-v0.2.0-macos.vst3` with binary SHA-256 `e0a8e2490b65255eff755c2b9e78e34a768ea947984f180762fa1edfca13bfa8`. Bitwig PID `32966` still maps the previously approved binary, so it must fully unload or restart before testing this final rebuild.
- The overlap-order fix moves playhead primitives after editable nodes and adds an exact phase-zero/default-endpoint regression. All 181 tests, focused VST3 coverage, warnings-denied clippy, artifact signing, and the final release build pass.

## Immediate Next Action

- Push the overlap-order follow-up to PR #13, wait for green checks, merge, then advance the audiodev submodule pointer.
