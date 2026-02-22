## Core Framework Boundary

- Reusable framework-level features must live in `toybox` when they can reasonably serve multiple plugins.
- Plugin repositories must stay focused on plugin-specific behavior only.
- Plugin-side widgets, DSP features, and UX/workflows are allowed only when they are absolutely specific to that plugin and there is no reasonable reuse path.
- If a widget, DSP feature, or workflow is reasonably reusable across plugins, it must be implemented in `toybox`, not in the plugin repo.
- From a plugin workspace, do not modify `toybox` directly.
- When reusable framework work is needed, write a clear, implementation-ready request for toybox developers and hand it off.
