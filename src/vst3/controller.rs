use super::*;
pub(super) struct PumpVst3Controller {
    shared: Arc<PumpVst3Shared>,
    component_handler: Mutex<Option<ComPtr<IComponentHandler>>>,
}

impl PumpVst3Controller {
    pub(super) fn new(shared: Arc<PumpVst3Shared>) -> Self {
        Self {
            shared,
            component_handler: Mutex::new(None),
        }
    }
}

impl Drop for PumpVst3Controller {
    fn drop(&mut self) {
        release_shared_for_role(&self.shared, SharedRole::Controller);
    }
}

impl Class for PumpVst3Controller {
    type Interfaces = (IEditController,);
}

impl IPluginBaseTrait for PumpVst3Controller {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

impl IEditControllerTrait for PumpVst3Controller {
    unsafe fn setComponentState(&self, state: *mut IBStream) -> tresult {
        let payload = unsafe { read_versioned_payload(state, STATE_MAGIC, &[STATE_VERSION]) };
        let Ok(payload) = payload else {
            return kInvalidArgument;
        };

        if decode_state_payload(self.shared.params.as_ref(), &payload.payload).is_ok() {
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        unsafe { self.setComponentState(state) }
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        let payload = encode_state_payload(self.shared.params.as_ref());
        match unsafe { write_versioned_payload(state, STATE_MAGIC, STATE_VERSION, &payload) } {
            Ok(()) => kResultOk,
            Err(_) => kResultFalse,
        }
    }

    unsafe fn getParameterCount(&self) -> int32 {
        param_count() as int32
    }

    unsafe fn getParameterInfo(&self, param_index: int32, info: *mut ParameterInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let Some(meta) = vst3_param_info_for_index(param_index) else {
            return kInvalidArgument;
        };

        let info = unsafe { &mut *info };
        info.id = meta.id;
        copy_wstring(meta.title, &mut info.title);
        copy_wstring(meta.short_title, &mut info.shortTitle);
        copy_wstring(meta.units, &mut info.units);
        info.stepCount = meta.step_count;
        info.defaultNormalizedValue = meta.default_normalized;
        info.unitId = 0;
        info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
        kResultOk
    }

    unsafe fn getParamStringByValue(
        &self,
        id: ParamID,
        value_normalized: ParamValue,
        string: *mut String128,
    ) -> tresult {
        if string.is_null() {
            return kInvalidArgument;
        }

        let Some(clap_id) = clap_id_from_vst3_param_id(id) else {
            return kInvalidArgument;
        };
        let plain = from_normalized(id, value_normalized);
        let Some(display) = format_plain_value_text(clap_id, plain) else {
            return kInvalidArgument;
        };
        copy_wstring(&display, unsafe { &mut *string });
        kResultOk
    }

    unsafe fn getParamValueByString(
        &self,
        id: ParamID,
        string: *mut TChar,
        value_normalized: *mut ParamValue,
    ) -> tresult {
        if value_normalized.is_null() {
            return kInvalidArgument;
        }

        let Some(clap_id) = clap_id_from_vst3_param_id(id) else {
            return kInvalidArgument;
        };
        let Some(raw) = (unsafe { parse_tchar_string(string) }) else {
            return kInvalidArgument;
        };
        let Some(plain) = parse_plain_value_text(clap_id, raw.trim()) else {
            return kInvalidArgument;
        };

        unsafe { *value_normalized = to_normalized(id, plain) };
        kResultOk
    }

    unsafe fn normalizedParamToPlain(
        &self,
        id: ParamID,
        value_normalized: ParamValue,
    ) -> ParamValue {
        from_normalized(id, value_normalized)
    }

    unsafe fn plainParamToNormalized(&self, id: ParamID, plain_value: ParamValue) -> ParamValue {
        to_normalized(id, plain_value)
    }

    unsafe fn getParamNormalized(&self, id: ParamID) -> ParamValue {
        to_normalized(id, read_plain_param(self.shared.params.as_ref(), id))
    }

    unsafe fn setParamNormalized(&self, id: ParamID, value: ParamValue) -> tresult {
        apply_normalized_param(self.shared.params.as_ref(), id, value);
        kResultOk
    }

    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        let Ok(mut component_handler) = self.component_handler.lock() else {
            return kResultFalse;
        };
        if handler.is_null() {
            *component_handler = None;
            return kResultOk;
        }
        unsafe { ((*(*handler).vtbl).base.addRef)(handler.cast()) };
        *component_handler = unsafe { ComPtr::from_raw(handler) };
        kResultOk
    }

    unsafe fn createView(&self, name: FIDString) -> *mut IPlugView {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = name;
            return ptr::null_mut();
        }

        #[cfg(target_os = "macos")]
        {
            if name.is_null() {
                return ptr::null_mut();
            }

            let requested = unsafe { CStr::from_ptr(name) };
            let editor = unsafe { CStr::from_ptr(ViewType::kEditor) };
            if requested.to_bytes() != editor.to_bytes() {
                return ptr::null_mut();
            }

            let adapter = PumpVst3GuiAdapter::new(self.shared.clone());
            let (default_width, default_height) = preferred_window_size();
            let Some(view) = ComWrapper::new(
                HostedVst3View::new(adapter, default_width, default_height).with_size_bounds(
                    MIN_WINDOW_WIDTH,
                    MIN_WINDOW_HEIGHT,
                    MAX_WINDOW_WIDTH,
                    MAX_WINDOW_HEIGHT,
                ),
            )
            .to_com_ptr::<IPlugView>() else {
                return ptr::null_mut();
            };
            ComPtr::into_raw(view)
        }
    }
}

unsafe fn parse_tchar_string(string: *mut TChar) -> Option<String> {
    if string.is_null() {
        return None;
    }
    let len = unsafe { tchar_len(string) };
    let utf16 = unsafe { slice::from_raw_parts(string.cast::<u16>(), len) };
    String::from_utf16(utf16).ok()
}
