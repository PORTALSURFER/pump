use super::*;
pub(super) struct PumpVst3GuiAdapter {
    radiant_gui: toybox::radiant_gui::RadiantHostedGui,
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ShortcutModifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl PumpVst3GuiAdapter {
    pub(super) fn new(shared: Arc<PumpVst3Shared>) -> Self {
        let editor = crate::gui::RadiantPumpEditor::new(
            Arc::clone(&shared.params),
            Arc::clone(&shared.status),
            Arc::clone(&shared.automation_queue),
            None,
            crate::gui::WINDOW_WIDTH,
            crate::gui::WINDOW_HEIGHT,
        );
        let radiant_gui = toybox::radiant_gui::RadiantHostedGui::new(
            "PumpRadiantVst3EditorView",
            editor,
            crate::gui::WINDOW_WIDTH,
            crate::gui::WINDOW_HEIGHT,
        )
        .with_size_contract(
            (crate::gui::MIN_WINDOW_WIDTH, crate::gui::MIN_WINDOW_HEIGHT),
            (crate::gui::WINDOW_WIDTH, crate::gui::WINDOW_HEIGHT),
            (crate::gui::MAX_WINDOW_WIDTH, crate::gui::MAX_WINDOW_HEIGHT),
        );
        Self { radiant_gui }
    }

    /// Decode VST3 modifier bit flags into Pump shortcut modifiers.
    ///
    /// Steinberg hosts commonly encode bitflags with shift/alt/control in the
    /// low bits. We accept both control-style bits to remain host-tolerant.
    #[cfg(test)]
    pub(super) fn shortcut_modifiers(modifiers: int16) -> ShortcutModifiers {
        let bits = modifiers as u16;
        ShortcutModifiers {
            shift: (bits & 0b0001) != 0,
            alt: (bits & 0b0010) != 0,
            ctrl: (bits & 0b0100) != 0 || (bits & 0b1000) != 0,
        }
    }

    /// Resolve a VST3 key event into one character/control input.
    #[cfg(test)]
    pub(super) fn key_char(key: char16, key_code: int16) -> Option<char> {
        toybox::vst3::gui::vst3_key_down_to_input_char(key, key_code)
    }
}

impl Vst3HostedGui for PumpVst3GuiAdapter {
    fn set_parent_raw(&mut self, parent: toybox::raw_window_handle::RawWindowHandle) {
        self.radiant_gui.set_parent(parent);
    }

    fn open(&mut self) -> bool {
        self.radiant_gui.open()
    }

    fn close(&mut self) {
        self.radiant_gui.close();
    }

    fn last_size(&self) -> Option<(u32, u32)> {
        self.radiant_gui.last_size()
    }

    fn request_resize(&self, width: u32, height: u32) {
        self.radiant_gui.request_resize(width, height);
    }

    fn on_key_down(&self, key: char16, key_code: int16, modifiers: int16) -> bool {
        self.radiant_gui.on_key_down(key, key_code, modifiers)
    }

    fn on_key_up(&self, key: char16, key_code: int16, modifiers: int16) -> bool {
        self.radiant_gui.on_key_up(key, key_code, modifiers)
    }
}
