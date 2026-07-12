# Active TODO

Ordered queue for immediate execution:

1. [x] Audit all Pump state and persistence collection decoders for serialized-count allocation hazards.
2. [x] Bound quick-slot, preset, curve-node, preset-name, and global-slot counts against semantic limits and minimum remaining bytes before allocation or iteration.
3. [x] Add host-state and preset-store malformed quick-slot coverage for `u32::MAX`, zero, below-required, above-limit, truncated, and valid counts without active-state mutation.
4. [x] Run default (194 tests) and VST3 (218 tests) CI, then build and audit the fresh signed review artifact.
5. [x] Open Pump PR #16 ready for review.
6. [ ] Wait for CI and explicit user review/sign-off on Pump PR #16.
