# Memory

- Last Updated (UTC): 2026-07-12 17:02:54 UTC
- Active Mission: Add the optional incoming-audio waveform background for OPT-1112.
- Current Workstream: Pump PR #19 (`https://github.com/PORTALSURFER/pump/pull/19`) is ready for review on `wsvasek/opt-1112-pump-optionally-show-the-incoming-waveform-or-kick-transient`. Scope: a user-controlled background waveform in both Pump curve editors, fixed-size lock-free phase-aligned capture, stable empty/unavailable handling, and disabled-path cost removal. Definition of Done: the issue's enabled, disabled, unavailable-input, alignment, render-order, realtime-bound, and no-audio-change requirements pass default, VST3, and screenshot validation. Status: waiting for user review; a fresh signed review artifact is available from the final PR head.

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
- OPT-1139 adds shared exact-boundary processing tests, CLAP timestamp adapter coverage, VST3 normalization/step coverage, and exact in-place VST3 channel-buffer coverage. Default CI passes 186 tests and VST3 CI passes 210 tests.
- The fresh signed OPT-1139 review artifact is `dist/pump-v0.2.0-macos.vst3` with binary SHA-256 `c49186c74dd8efdf59f44fd7b709d8eb2b76658c9e59191e713632c5ca2f509c`; signature, plist, arm64 Mach-O, and VST3 entry-symbol audits pass. Bitwig PID `32966` still maps the previous binary and must fully unload or restart before the large-buffer host smoke test.
- OPT-1141 rejects host-state and preset-store quick-slot counts unless they equal `QUICK_SLOT_COUNT`, validates minimum remaining bytes before all state/persistence collection allocations, and applies the same rule to the global curve-slot store sibling decoder.
- OPT-1141 default CI passes 194 tests and VST3 CI passes 218 tests. The fresh signed review artifact is `dist/pump-v0.2.0-macos.vst3` with binary SHA-256 `225509e1e00bb6f42fa9069327d5d88895e734bbeaf190998480f2a8bfc80c37`; signature, plist, arm64 Mach-O, and VST3 entry-symbol audits pass. Bitwig PID `32966` still maps the previous binary and must fully unload or restart before testing.
- OPT-1140 preallocates both CLAP stereo scratch vectors to `max_frames_count`, reserves the bounded Toybox automation queue capacity in the audio-thread drain vector, and caps CLAP parameter scheduling at four points per declared frame without growing in `process`.
- Oversized CLAP host blocks apply parameter changes, silence writable outputs, and drain outgoing automation without allocating, panicking, returning an error, or triggering realtime logging.
- OPT-1140 default CI passes 200 tests and VST3 CI passes 224 tests. CLAP host smoke passes at 16, 64, 512, and 2048 frames with zero xruns. The fresh signed review artifact is `dist/pump-v0.2.0-macos.vst3` with binary SHA-256 `bb00d2a5c52701cb3fb9e061409d11a7c3188079badbb4357ab50fc74aa69e20`; signature, plist, arm64 Mach-O, and VST3 entry-symbol audits pass.
- Pump PR #17 for OPT-1140 merged at `b5779611772b728cccd041c3dda953541dec48c8`; its CI checks passed and no active PR remained before OPT-1142 began.
- OPT-1142 stages create, overwrite, rename, full-bank replacement, legacy preset quick-slot, and selected-index changes, persists the candidate first, and commits runtime state only after success. Failed preset selection also leaves active parameters unchanged.
- `PresetMutationError` separates invalid index/name, capacity, state access, and persistence failures. A persistence failure remains visible as `NOT SAVED - CHECK PRESET FOLDER` in both Toybox and Radiant until a later preset-bank write succeeds.
- Preset persistence tests inject directory-create, temporary-write, and final-rename failures; assert rollback and reload state; exercise a genuinely unwritable directory; and verify the existing durable bank remains readable after finalization failure.
- OPT-1142 default CI passes 206 tests and VST3 CI passes 230 tests. The fresh signed review artifact is `dist/pump-v0.2.0-macos.vst3` with binary SHA-256 `3deeaa2b9dfd4e765ea1362528013fdf30a34e78d9d30b2ae91cda6f0fd50667`; signature, plist, arm64 Mach-O, and VST3 entry-symbol audits pass.
- The PR #18 undo review fix only persists a history snapshot's preset bank when it differs from the current bank, so knob and curve undo/redo remain available during preset-store failures. Failed preset-history persistence leaves the history entry available for retry.
- The real unwritable-directory regression probes write capability after applying Unix mode `0500`; environments such as UID 0 that can still create a file clean up and skip, while permission-enforcing environments exercise the real persistence failure.
- Toybox renders preset persistence failure as an independent expanded header status, so `NOT SAVED - CHECK PRESET FOLDER` stays visible alongside an active rename textbox; the regression asserts both warning and rename draft in the same UI frame.
- Pump PR #18 for OPT-1142 merged at `29474c9`; the branch cleanup and generated changelog left Pump `main` at `b115b00` before OPT-1112 began.
- OPT-1112 adds a disabled-by-default `Wave` / `Input waveform` toggle in the Toybox and Radiant editors. Enabled capture aggregates pre-gain stereo peaks into 96 atomic bins keyed by the exact DSP cycle phase; generation changes remove stale bins on enable, disable, unavailable input, and cycle wrap without clearing arrays on the audio thread.
- Incoming-waveform rendering is a low-contrast symmetric envelope behind the editable curve, nodes, playhead, and interaction feedback. Default CI passes 216 tests, VST3 CI passes 241 tests, and screenshot validation passes at 315x211, 420x282, 525x352, and 630x423.
- The OPT-1112 review artifact is `dist/pump-v0.2.0-macos.vst3`; signing, plist, arm64 Mach-O, and VST3 entry-symbol audits pass. The final binary hash is recorded on PR #19 after the last documentation commit and rebuild.

## Immediate Next Action

- Wait for GitHub CI and explicit user review/sign-off on ready-for-review Pump PR #19.
