# Memory

- Last Updated (UTC): 2026-02-22 12:21:07 UTC
- Active Mission: Keep routine cleanup and handoff quality high without expanding scope.
- Current Workstream: One-shot code-quality cleanup completed with green local CI.

## Current State

- `AGENTS.md` is a portal that points to current-state and plan files.
- Active execution order lives in `docs/plans/active/todo.md`.
- Detailed plan navigation lives in `docs/plans/index.md`.
- Local preflight entrypoint is `scripts/run_agent_request.sh`.
- Local changelog generator is `scripts/update_changelog.sh`.
- Push-time changelog updater is `.github/workflows/changelog.yml`.
- Shared monotonic timing utility is `src/time_utils.rs`.

## Immediate Next Action

- Continue from `docs/plans/active/todo.md` item `1` for the next request.
