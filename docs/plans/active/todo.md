# Active TODO

Ordered queue for immediate execution:

1. [x] Resolve the OPT-1116 architecture boundary and create implementation-ready Toybox prerequisite OPT-1173.
2. [x] Add Radiant/VST3 stable Shift gain anchoring, mid-gesture engage/release, and cleanup state.
3. [x] Cover Radiant Shift-from-start, vertical drift, no-jump release, ordering/push-through boundaries, wrapped endpoints, Shift+Command point precedence, and consecutive gestures.
4. [x] Pass focused default/VST3 Radiant tests and warnings-denied VST3 clippy.
5. [x] Land Toybox OPT-1173 and repin Pump to merged revision `c99ab4205984e342bc97288ab0f6f431723604a5`.
6. [x] Opt the declarative Pump editor into the reusable Shift horizontal point constraint.
7. [ ] Integrate and verify the OPT-1115 Command beat-grid snap path so Shift+Command composes exactly as specified.
8. [x] Run full default CI, VST3 CI, and multi-size release screenshot validation.
9. [ ] Commit, push, and open the OPT-1116 Pump PR ready for review.
10. [ ] Build and audit the exact-head signed `dist/pump-v0.2.0-macos.vst3` review artifact.
11. [ ] Wait for explicit user review/sign-off before merge.
