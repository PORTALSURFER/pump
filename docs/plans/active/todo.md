# Active TODO

Ordered queue for immediate execution:

1. [x] Publish strongest per-block Pump-envelope attenuation from non-silent input without audio-thread allocation or blocking.
2. [x] Add bounded dB mapping, clamping, fast-attack/slow-release ballistics, and stable stopped/silent/missing/stale behavior.
3. [x] Place a compact labeled meter beside the Toybox and Radiant curve editors without changing DSP, automation, curve evaluation, or interaction semantics.
4. [x] Add focused value-mapping, clamping, inactive-state, block aggregation, output-trim isolation, repaint, and dual-renderer paint tests.
5. [x] Pass default CI, VST3 CI, and four-size idle/live screenshot validation.
6. [ ] Commit and open the OPT-1114 PR ready for review.
7. [ ] Build and audit the signed `dist/pump-v0.2.0-macos.vst3` review artifact; record its exact-head SHA-256 on the PR.
8. [ ] Wait for GitHub CI and explicit user review/sign-off on the OPT-1114 PR.
