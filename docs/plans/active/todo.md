# Active TODO

Ordered queue for immediate execution:

1. [x] Preallocate CLAP stereo scratch, automation drain, and bounded parameter-event storage during activation.
2. [x] Make normal CLAP process/flush paths allocation-free and silence host blocks that exceed `max_frames_count`.
3. [x] Add allocator-guarded first/max/separate-buffer, dense-event, in-place, oversize, and full-automation-queue coverage.
4. [x] Run default CI (200 tests), VST3 CI (224 tests), and CLAP host smoke at 16, 64, 512, and 2048 frames.
5. [x] Build and audit the signed `dist/pump-v0.2.0-macos.vst3` review artifact.
6. [x] Open Pump PR #17 ready for review.
7. [ ] Wait for CI and explicit user review/sign-off on the OPT-1140 PR.
