# Memory

- Last Updated (UTC): 2026-06-28 14:23:15 UTC
- Active Mission: Add OPT-927 Cmd-click numeric entry to Pump parameter value labels.
- Current Workstream: Branch `wsvasek/opt-927-pump-add-cmd-click-numeric-entry-for-parameter-value-labels` makes Radiant/VST3 value labels enter numeric keyboard-edit mode on macOS command-click while preserving normal click/drag behavior for regular interactions.

## Current State

- `AGENTS.md` is a portal that points to current-state and plan files.
- Active execution order lives in `docs/plans/active/todo.md`.
- Detailed plan navigation lives in `docs/plans/index.md`.
- Local preflight entrypoint is `scripts/run_agent_request.sh`.
- Local changelog generator is `scripts/update_changelog.sh`.
- Push-time changelog updater is `.github/workflows/changelog.yml`.
- Shared monotonic timing utility is `src/time_utils.rs`.
- Mix, Phase, and Output value labels now use a Radiant numeric-entry label widget that begins editing only on command-click.
- Numeric entry drafts start from the existing host-facing plain value text, accept typed numeric/unit characters, commit on Enter, cancel on Escape or focus loss, and leave the parameter unchanged for invalid commits.
- Numeric entry commits reuse `params::host_api` formatting/parsing and send the same automation begin/value/end queue events as normal UI edits.
- The Sync value label remains passive because the control is enumerated/non-numeric in this UI.
- The macOS VST3 AppKit editor view now becomes first responder on mouse down and forwards keyDown text, Enter, Escape, Backspace, and Delete into the Radiant runtime.
- `bash scripts/update_changelog.sh`, `bash scripts/ci_local.sh`, `bash scripts/ci.sh`, focused Radiant/AppKit/VST3 text tests, and `VST3_SDK_DIR=/Users/portalsurfer/lib/vst3sdk cargo test --features vst3` are green.

## Immediate Next Action

- Build the fresh host-installable Pump VST3 from the audiodev superproject, commit and push the OPT-927 Pump branch, then sync the updated submodule pointer in the audiodev superproject PR.
