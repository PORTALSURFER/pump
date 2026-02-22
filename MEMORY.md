# Memory

- Last Updated (UTC): 2026-02-22 11:05:28 UTC
- Active Mission: Keep changelog generation automatic and deterministic on every push to `main`.
- Current Workstream: Changelog automation is active through local script + GitHub workflow.

## Current State

- `AGENTS.md` is a portal that points to current-state and plan files.
- Active execution order lives in `docs/plans/active/todo.md`.
- Detailed plan navigation lives in `docs/plans/index.md`.
- Local preflight entrypoint is `scripts/run_agent_request.sh`.
- Local changelog generator is `scripts/update_changelog.sh`.
- Push-time changelog updater is `.github/workflows/changelog.yml`.

## Immediate Next Action

- Run `bash scripts/update_changelog.sh` before pushing, then rely on push workflow for final sync.
