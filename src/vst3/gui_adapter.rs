use super::*;
pub(super) struct PumpVst3GuiAdapter {
    shared: Arc<PumpVst3Shared>,
    gui: PumpGui,
    #[cfg(target_os = "macos")]
    cocoa_gui: super::cocoa_gui::CocoaPumpEditor,
}

impl PumpVst3GuiAdapter {
    pub(super) fn new(shared: Arc<PumpVst3Shared>) -> Self {
        Self {
            shared,
            gui: PumpGui::default(),
            #[cfg(target_os = "macos")]
            cocoa_gui: super::cocoa_gui::CocoaPumpEditor::default(),
        }
    }

    /// Decode VST3 modifier bit flags into Pump shortcut modifiers.
    ///
    /// Steinberg hosts commonly encode bitflags with shift/alt/control in the
    /// low bits. We accept both control-style bits to remain host-tolerant.
    #[cfg(any(not(target_os = "macos"), test))]
    pub(super) fn shortcut_modifiers(modifiers: int16) -> toybox::clap::gui::ShortcutModifiers {
        let bits = modifiers as u16;
        toybox::clap::gui::ShortcutModifiers::new(
            (bits & 0b0001) != 0,
            (bits & 0b0010) != 0,
            (bits & 0b0100) != 0 || (bits & 0b1000) != 0,
        )
    }

    /// Resolve a VST3 key event into one character/control input.
    #[cfg(any(not(target_os = "macos"), test))]
    pub(super) fn key_char(key: char16, key_code: int16) -> Option<char> {
        toybox::vst3::gui::vst3_key_down_to_input_char(key, key_code)
    }

    #[cfg(not(target_os = "macos"))]
    fn should_consume_key(
        &self,
        ch: char,
        modifiers: toybox::clap::gui::ShortcutModifiers,
    ) -> bool {
        self.gui.text_edit_active()
            || self.gui.shortcut_action_for_input(ch, modifiers).is_some()
            || ch.eq_ignore_ascii_case(&crate::gui::SHORTCUT_KEY_SNAP_INVERT)
    }
}

impl Vst3HostedGui for PumpVst3GuiAdapter {
    fn set_parent_raw(&mut self, parent: toybox::raw_window_handle::RawWindowHandle) {
        #[cfg(target_os = "macos")]
        self.cocoa_gui.set_parent_raw(parent);
        self.gui.set_parent_raw(parent);
    }

    fn open(&mut self) -> bool {
        #[cfg(target_os = "macos")]
        {
            let _ = &self.shared.automation_queue;
            self.cocoa_gui.open(
                Arc::clone(&self.shared.params),
                Arc::clone(&self.shared.status),
            )
        }

        #[cfg(not(target_os = "macos"))]
        {
            self.gui
                .open(
                    &self.shared.params,
                    &self.shared.status,
                    self.shared.automation_queue.clone(),
                    None,
                )
                .is_ok()
        }
    }

    fn close(&mut self) {
        #[cfg(target_os = "macos")]
        self.cocoa_gui.close();
        self.gui.close();
    }

    fn last_size(&self) -> Option<(u32, u32)> {
        #[cfg(target_os = "macos")]
        {
            self.cocoa_gui.last_size()
        }

        #[cfg(not(target_os = "macos"))]
        {
            self.gui.last_size()
        }
    }

    fn request_resize(&self, width: u32, height: u32) {
        #[cfg(target_os = "macos")]
        {
            self.cocoa_gui.request_resize(width, height);
        }

        #[cfg(not(target_os = "macos"))]
        {
            self.gui.request_resize(width, height);
        }
    }

    fn on_key_down(&self, key: char16, key_code: int16, modifiers: int16) -> bool {
        #[cfg(target_os = "macos")]
        {
            let _ = (key, key_code, modifiers);
            false
        }

        #[cfg(not(target_os = "macos"))]
        {
            let Some(ch) = Self::key_char(key, key_code) else {
                return false;
            };
            let shortcut_modifiers = Self::shortcut_modifiers(modifiers);
            if !self.should_consume_key(ch, shortcut_modifiers) {
                return false;
            }
            self.gui.post_injected_text_char(ch, shortcut_modifiers)
        }
    }

    fn on_key_up(&self, key: char16, key_code: int16, modifiers: int16) -> bool {
        #[cfg(target_os = "macos")]
        {
            let _ = (key, key_code, modifiers);
            false
        }

        #[cfg(not(target_os = "macos"))]
        {
            let Some(ch) = Self::key_char(key, key_code) else {
                return false;
            };
            let shortcut_modifiers = Self::shortcut_modifiers(modifiers);
            if !self.should_consume_key(ch, shortcut_modifiers) {
                return false;
            }
            self.gui.post_injected_key_up(ch, shortcut_modifiers)
        }
    }
}
