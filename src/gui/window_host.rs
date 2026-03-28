//! Host-window integration for the Pump GUI.
//!
//! This module keeps native host-window plumbing separate from declarative UI
//! state/layout logic so GUI behavioral changes and host adapter changes remain
//! isolated during review.

use super::*;

/// Host-window wrapper for the Pump editor.
#[derive(Default)]
pub struct PumpGui {
    pub(super) window: GuiHostWindow,
}

impl PumpGui {
    /// Return default focused-window keyboard shortcuts for Pump.
    fn default_shortcuts() -> Vec<ShortcutBinding> {
        vec![
            ShortcutBinding::new(
                PRESET_RENAME_BUTTON_KEY,
                SHORTCUT_KEY_RENAME,
                ShortcutModifiers::default(),
            ),
            ShortcutBinding::new(
                PRESET_SAVE_KEY,
                SHORTCUT_KEY_SAVE,
                ShortcutModifiers::default(),
            ),
            ShortcutBinding::new(UNDO_KEY, SHORTCUT_KEY_UNDO, ShortcutModifiers::default()),
            ShortcutBinding::new(
                REDO_KEY,
                SHORTCUT_KEY_UNDO,
                ShortcutModifiers::new(true, false, false),
            ),
            ShortcutBinding::new(
                PRESET_ADD_KEY,
                SHORTCUT_KEY_ADD,
                ShortcutModifiers::new(true, false, false),
            ),
            ShortcutBinding::new(
                PRESET_ADD_KEY,
                SHORTCUT_KEY_ADD_ALT,
                ShortcutModifiers::default(),
            ),
        ]
    }

    /// Attach raw host window handle.
    pub fn set_parent_raw(&mut self, parent: RawWindowHandle) {
        self.window.set_parent(parent);
    }

    /// Attach CLAP host parent window.
    pub fn set_parent(&mut self, window: Window<'_>) {
        self.set_parent_raw(window.raw_window_handle());
    }

    /// Open Pump editor.
    pub fn open(
        &mut self,
        params: &Arc<PumpParams>,
        status: &Arc<GuiStatus>,
        automation_queue: Arc<AutomationQueue>,
        param_requester: Option<HostParamRequester>,
    ) -> Result<(), PluginError> {
        self.window.set_aspect_ratio(Some(DESIGN_ASPECT_RATIO));
        self.window.set_shortcuts(Self::default_shortcuts());
        let state = GuiState::new(
            Arc::clone(params),
            Arc::clone(status),
            automation_queue,
            param_requester,
        );
        let open_size = state.measured_open_size();
        let on_init = Box::new(|_state: &mut GuiState| {});
        let build = Box::new(|input: &InputState, state: &GuiState| state.build_ui(input));
        let reduce = Box::new(|state: &mut GuiState, action: UiAction| state.reduce_action(action));

        self.window
            .open_parented_with(GuiOpenRequest::<GuiState, _, _, _>::new(
                "pump".to_string(),
                open_size,
                state,
                on_init,
                build,
                reduce,
            ))
    }

    /// Request a logical resize from the GUI thread.
    #[cfg(any(feature = "vst3", windows))]
    #[allow(dead_code)]
    pub fn request_resize(&self, width: u32, height: u32) {
        self.window.request_resize(width, height);
    }

    /// Inject one character tagged as host-injected key input.
    #[cfg(any(feature = "vst3", windows))]
    #[allow(dead_code)]
    pub fn post_injected_text_char(&self, ch: char, modifiers: ShortcutModifiers) -> bool {
        self.window.post_injected_text_char(ch, modifiers)
    }

    /// Return `true` when preset rename text editing is active.
    #[cfg(any(feature = "vst3", windows))]
    #[allow(dead_code)]
    pub fn text_edit_active(&self) -> bool {
        self.window.text_edit_active()
    }

    /// Resolve one registered shortcut action key from input.
    #[cfg(any(feature = "vst3", windows))]
    #[allow(dead_code)]
    pub fn shortcut_action_for_input(
        &self,
        ch: char,
        modifiers: ShortcutModifiers,
    ) -> Option<String> {
        self.window.shortcut_action_for_input(ch, modifiers)
    }

    /// Return true when host-driven resizing is enabled.
    pub fn host_resize_enabled(&self) -> bool {
        self.window.host_resize_enabled()
    }

    /// Resolve host-adjusted size according to the configured resize policy.
    pub fn adjust_host_size(&self, size: GuiSize) -> Option<GuiSize> {
        self.window
            .adjust_host_size(size)
            .map(constrained_host_size)
    }

    /// Apply a host-provided size using Toybox's canonical resize behavior.
    pub fn apply_host_size(&self, size: GuiSize) {
        self.window.apply_host_size(constrained_host_size(size));
    }

    /// Close editor if it is open.
    pub fn close(&mut self) {
        self.window.hide();
    }

    /// Return last known logical size.
    pub fn last_size(&self) -> Option<(u32, u32)> {
        self.window.last_size()
    }
}

/// Return the preferred logical Pump window size measured from declarative layout.
///
/// Hosts may query a plugin view size before the GUI is opened. This helper
/// provides a stable measured fallback so host-side parent windows are large
/// enough for the current declarative content on first attach.
pub(crate) fn preferred_window_size() -> (u32, u32) {
    static PREFERRED_SIZE: OnceLock<(u32, u32)> = OnceLock::new();
    *PREFERRED_SIZE.get_or_init(|| {
        let state = GuiState::new(
            Arc::new(PumpParams::new()),
            Arc::new(GuiStatus::default()),
            Arc::new(AutomationQueue::default()),
            None,
        );
        state.measured_open_size()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shortcuts_include_undo_and_redo_bindings() {
        let shortcuts = PumpGui::default_shortcuts();
        assert!(shortcuts.iter().any(|binding| {
            binding.action_key == UNDO_KEY
                && binding.matches('u', ShortcutModifiers::default())
                && binding.matches('U', ShortcutModifiers::default())
        }));
        assert!(shortcuts.iter().any(|binding| {
            binding.action_key == REDO_KEY
                && binding.matches('u', ShortcutModifiers::new(true, false, false))
                && binding.matches('U', ShortcutModifiers::new(true, false, false))
        }));
    }
}
