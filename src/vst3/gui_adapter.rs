use super::*;
pub(super) struct PumpVst3GuiAdapter {
    shared: Arc<PumpVst3Shared>,
    gui: PumpGui,
}

impl PumpVst3GuiAdapter {
    pub(super) fn new(shared: Arc<PumpVst3Shared>) -> Self {
        Self {
            shared,
            gui: PumpGui::default(),
        }
    }

    /// Decode VST3 modifier bit flags into Pump shortcut modifiers.
    ///
    /// Steinberg hosts commonly encode bitflags with shift/alt/control in the
    /// low bits. We accept both control-style bits to remain host-tolerant.
    pub(super) fn shortcut_modifiers(modifiers: int16) -> toybox::clap::gui::ShortcutModifiers {
        let bits = modifiers as u16;
        toybox::clap::gui::ShortcutModifiers::new(
            (bits & 0b0001) != 0,
            (bits & 0b0010) != 0,
            (bits & 0b0100) != 0 || (bits & 0b1000) != 0,
        )
    }

    /// Resolve a VST3 key event into one character/control input.
    pub(super) fn key_char(key: char16, key_code: int16) -> Option<char> {
        toybox::vst3::gui::vst3_key_down_to_input_char(key, key_code)
    }
}

impl Vst3HostedGui for PumpVst3GuiAdapter {
    fn set_parent_raw(&mut self, parent: toybox::raw_window_handle::RawWindowHandle) {
        self.gui.set_parent_raw(parent);
    }

    fn open(&mut self) -> bool {
        self.gui
            .open(
                &self.shared.params,
                &self.shared.status,
                self.shared.automation_queue.clone(),
                None,
            )
            .is_ok()
    }

    fn close(&mut self) {
        self.gui.close();
    }

    fn last_size(&self) -> Option<(u32, u32)> {
        self.gui.last_size()
    }

    fn request_resize(&self, width: u32, height: u32) {
        self.gui.request_resize(width, height);
    }

    fn on_key_down(&self, key: char16, key_code: int16, modifiers: int16) -> bool {
        let Some(ch) = Self::key_char(key, key_code) else {
            return false;
        };
        let shortcut_modifiers = Self::shortcut_modifiers(modifiers);
        let should_consume = self.gui.text_edit_active()
            || self
                .gui
                .shortcut_action_for_input(ch, shortcut_modifiers)
                .is_some();
        if !should_consume {
            return false;
        }
        self.gui.post_injected_text_char(ch, shortcut_modifiers)
    }
}
