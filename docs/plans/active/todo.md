# Active TODO

Ordered queue for immediate execution:

1. [x] Derive normalized major and minor curve-grid positions from each supported Pump sync length.
2. [x] Apply the shared timing model to both the Toybox and Radiant curve editors without changing snapping or interaction semantics.
3. [x] Thin minor divisions at narrow widths and define stable empty behavior for unsupported timing and the boundary-free 1/16 cycle.
4. [x] Add focused sync-length, resize, alignment, full-height geometry, and unsupported-state tests.
5. [x] Pass default CI (229 tests), VST3 CI (256 tests), and four-size screenshot validation with and without the playhead.
6. [x] Commit and open Pump PR #20 ready for review.
7. [x] Build and audit the signed `dist/pump-v0.2.0-macos.vst3` review artifact (binary SHA-256 `7da1e9fa5cc13d012e962c4c627c2563f9f82bfe5204a9df445e5181224590e8`).
8. [ ] Wait for GitHub CI and explicit user review/sign-off on the OPT-1111 PR.
