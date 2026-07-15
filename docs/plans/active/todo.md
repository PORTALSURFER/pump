# Active TODO

Ordered queue for immediate execution:

1. [x] Verify Toybox PR #11 merged, canonicalize Toybox `main`, validate its ledger/post-merge state, and close OPT-1176.
2. [x] Revise OPT-1117's Linear contract so its required Shift-only transition explicitly integrates the preserved OPT-1116 Pump work.
3. [x] Create `wsvasek/opt-1117-pump-hold-shiftoption-to-constrain-curve-point-movement` from current Pump `main`.
4. [x] Reconcile the preserved Radiant Shift-only implementation with current Command snapping.
5. [x] Repin Pump to canonical Toybox `428d6a637cddf6906f09832e2426bb428fbdfd8a` and enable both point-constraint decorators.
6. [x] Add Radiant/VST3 Shift+Option stable time anchoring, no-jump transitions, and Cmd precedence.
7. [x] Cover start/mid-gesture transitions, pointer drift, Shift-only composition, Cmd composition, boundaries, focus/cancel, release, and consecutive gestures.
8. [x] Pass focused tests, full default/VST3 CI, warnings-denied clippy, and multi-size rendered visual validation.
9. [x] Build, sign, audit, and host-smoke the exact-head `dist/pump-v0.2.0-macos.vst3` artifact.
10. [x] Commit, push, open ready Pump PR #25, verify remote readback, and validate the contract-revision-2 review ledger.
11. [ ] Address review findings on the same branch, resolve their GitHub threads after exact-head verification, and run a fresh complete-diff review until clean.
12. [ ] Wait for explicit user review/sign-off before merge.
