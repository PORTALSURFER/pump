use super::*;

pub(super) struct Vst3HostParamEditSink {
    pub(super) shared: Arc<PumpVst3Shared>,
}

fn vst3_param_id_for_gui(param_id: toybox::clack_plugin::utils::ClapId) -> ParamID {
    if param_id == PARAM_SYNC_DIVISION_ID {
        PARAM_SYNC_DIVISION_VST3_V2_NUM
    } else {
        param_id.get()
    }
}

impl crate::gui::HostParamEditSink for Vst3HostParamEditSink {
    fn edit(
        &self,
        config: &toybox::clap::automation::AutomationConfig,
        param_id: toybox::clack_plugin::utils::ClapId,
        value: f64,
    ) -> bool {
        if !config.is_enabled(param_id) {
            return false;
        }
        let vst3_id = vst3_param_id_for_gui(param_id);
        let Some(normalized) = normalized_from_vst3_plain_value(vst3_id, value) else {
            return false;
        };
        let Ok(handler) = self.shared.component_handler.lock() else {
            return false;
        };
        let Some(handler) = handler.as_ref() else {
            return false;
        };
        unsafe {
            if handler.beginEdit(vst3_id) != kResultOk {
                return false;
            }
            let performed = handler.performEdit(vst3_id, normalized) == kResultOk;
            let _ended = handler.endEdit(vst3_id);
            performed
        }
    }

    fn gesture_started(
        &self,
        config: &toybox::clap::automation::AutomationConfig,
        param_id: toybox::clack_plugin::utils::ClapId,
    ) -> bool {
        if !config.is_enabled(param_id) {
            return false;
        }
        let vst3_id = vst3_param_id_for_gui(param_id);
        let Ok(handler) = self.shared.component_handler.lock() else {
            return false;
        };
        let Some(handler) = handler.as_ref() else {
            return false;
        };
        unsafe { handler.beginEdit(vst3_id) == kResultOk }
    }

    fn gesture_value(
        &self,
        config: &toybox::clap::automation::AutomationConfig,
        param_id: toybox::clack_plugin::utils::ClapId,
        value: f64,
    ) -> bool {
        if !config.is_enabled(param_id) {
            return false;
        }
        let vst3_id = vst3_param_id_for_gui(param_id);
        let Some(normalized) = normalized_from_vst3_plain_value(vst3_id, value) else {
            return false;
        };
        let Ok(handler) = self.shared.component_handler.lock() else {
            return false;
        };
        let Some(handler) = handler.as_ref() else {
            return false;
        };
        unsafe { handler.performEdit(vst3_id, normalized) == kResultOk }
    }

    fn gesture_ended(
        &self,
        config: &toybox::clap::automation::AutomationConfig,
        param_id: toybox::clack_plugin::utils::ClapId,
    ) -> bool {
        if !config.is_enabled(param_id) {
            return false;
        }
        let vst3_id = vst3_param_id_for_gui(param_id);
        let Ok(handler) = self.shared.component_handler.lock() else {
            return false;
        };
        let Some(handler) = handler.as_ref() else {
            return false;
        };
        unsafe { handler.endEdit(vst3_id) == kResultOk }
    }
}

pub(super) struct PumpVst3GuiAdapter {
    gpui_gui: toybox::gpui_gui::GpuiHostedGui,
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
        let gpui_gui = crate::gui::gui_gpui::new_gui_with_edit_sink(
            Arc::clone(&shared.params),
            Arc::clone(&shared.status),
            Arc::new(Vst3HostParamEditSink {
                shared: Arc::clone(&shared),
            }),
        );
        Self { gpui_gui }
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
        self.gpui_gui.set_parent_raw(parent);
    }

    fn open(&mut self) -> bool {
        self.gpui_gui.open()
    }

    fn close(&mut self) {
        self.gpui_gui.close();
    }

    fn last_size(&self) -> Option<(u32, u32)> {
        self.gpui_gui.last_size()
    }

    fn show(&self) -> bool {
        self.gpui_gui.show()
    }

    fn set_callback_keyboard_mode(&mut self, callback_only: bool) {
        self.gpui_gui.set_callback_keyboard_mode(callback_only);
    }

    fn host_size_from_logical(&self, width: u32, height: u32) -> (u32, u32) {
        self.gpui_gui.host_size_from_logical(width, height)
    }

    fn logical_size_from_host(&self, width: u32, height: u32) -> (u32, u32) {
        self.gpui_gui.logical_size_from_host(width, height)
    }

    fn request_resize(&self, width: u32, height: u32) {
        self.gpui_gui.request_resize(width, height);
    }

    fn on_key_down(&self, key: char16, key_code: int16, modifiers: int16) -> bool {
        self.gpui_gui.on_key_down(key, key_code, modifiers)
    }

    fn on_key_up(&self, key: char16, key_code: int16, modifiers: int16) -> bool {
        self.gpui_gui.on_key_up(key, key_code, modifiers)
    }

    fn on_focus(&self, focused: bool) -> bool {
        self.gpui_gui.on_focus(focused)
    }
}
