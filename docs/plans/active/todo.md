# Active TODO

Ordered queue for immediate execution:

1. [x] Add disabled-by-default fixed-size incoming-audio capture keyed to Pump's normalized cycle phase.
2. [x] Invalidate stale data for disable, unavailable/silent input, and cycle wrap without blocking or allocating on the audio thread.
3. [x] Add explicit waveform toggles and subordinate background rendering to the Toybox and Radiant curve editors.
4. [x] Cover enabled, disabled, unavailable-input, alignment, render order, and allocation-free processing states.
5. [x] Pass default CI (224 tests), VST3 CI (251 tests), and four-size screenshot validation, including backward/forward seeks, cycle-mapping changes, empty blocks, all-silent blocks, and CLAP zero-frame/output-only input classification.
6. [x] Commit and open Pump PR #19 ready for review.
7. [x] Build and audit the signed `dist/pump-v0.2.0-macos.vst3` review artifact.
8. [ ] Wait for GitHub CI and explicit user review/sign-off on the OPT-1112 PR.
