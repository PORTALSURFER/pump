# Active TODO

Ordered queue for immediate execution:

1. [x] Supersede and preserve the incomplete OPT-1116 workstream before starting a separate PR.
2. [x] Create the dedicated OPT-1115 branch from updated Pump `main` and move Linear to In Progress.
3. [x] Derive deterministic time snap targets from the exact visible sync-aware beat grid, including boundaries and stable tie-breaking.
4. [x] Enable Command time-only snapping for declarative insertion and point dragging without changing gain or legacy Snap/S-key behavior.
5. [x] Add matching Radiant/VST3 insertion, point dragging, and mid-gesture Command transition behavior.
6. [x] Cover short/long sync lengths, insertion, drag transitions, vertical continuity, boundaries, and existing Command segment precedence.
7. [x] Pass full default CI, VST3 CI, and multi-size release screenshot validation.
8. [x] Commit, push, and open OPT-1115 Pump PR #24 ready for review; explicit merge authorization supplies sign-off.
9. [ ] Build and audit the exact-head signed `dist/pump-v0.2.0-macos.vst3` artifact.
10. [ ] Verify GitHub CI, merge under explicit user authorization, clean branches, and mark Linear Done.
