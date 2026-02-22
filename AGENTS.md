# Agent Wake-Up Portal

Use this file as a fast entrypoint. Keep details in `docs/` and `MEMORY.md`.

## 60-Second Wake-Up

1. Run `bash scripts/run_agent_request.sh`.
2. Read `MEMORY.md` for current state.
3. Open `docs/plans/active/todo.md` and execute item `1`.

## Source of Truth

- Current status: `MEMORY.md`
- Active queue: `docs/plans/active/todo.md`
- Plan map: `docs/plans/index.md`
- Documentation map: `docs/README.md`
- Changelog automation: `scripts/update_changelog.sh` and `.github/workflows/changelog.yml`

## Core Framework Boundary

- Reusable framework-level features must live in `toybox` when they can reasonably serve multiple plugins.
- Plugin repositories must stay focused on plugin-specific behavior only.
- Plugin-side widgets, DSP features, and UX/workflows are allowed only when they are absolutely specific to that plugin and there is no reasonable reuse path.
- If a widget, DSP feature, or workflow is reasonably reusable across plugins, it must be implemented in `toybox`, not in the plugin repo.
- From a plugin workspace, do not modify `toybox` directly.
- When reusable framework work is needed, write a clear, implementation-ready request for toybox developers and hand it off.
