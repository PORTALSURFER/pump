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
        5
    }

    unsafe fn getParameterInfo(&self, param_index: int32, info: *mut ParameterInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let info = unsafe { &mut *info };
        match param_index {
            0 => {
                info.id = PARAM_MIX_ID;
                copy_wstring("Mix", &mut info.title);
                copy_wstring("Mix", &mut info.shortTitle);
                copy_wstring("%", &mut info.units);
                info.stepCount = 0;
                info.defaultNormalizedValue = to_normalized(PARAM_MIX_ID, DEFAULT_MIX as f64);
                info.unitId = 0;
                info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
                kResultOk
            }
            1 => {
                info.id = PARAM_DEPTH_ID;
                copy_wstring("Depth", &mut info.title);
                copy_wstring("Depth", &mut info.shortTitle);
                copy_wstring("%", &mut info.units);
                info.stepCount = 0;
                info.defaultNormalizedValue = to_normalized(PARAM_DEPTH_ID, DEFAULT_DEPTH as f64);
                info.unitId = 0;
                info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
                kResultOk
            }
            2 => {
                info.id = PARAM_PHASE_OFFSET_ID;
                copy_wstring("Phase Offset", &mut info.title);
                copy_wstring("Phase", &mut info.shortTitle);
                copy_wstring("%", &mut info.units);
                info.stepCount = 0;
                info.defaultNormalizedValue =
                    to_normalized(PARAM_PHASE_OFFSET_ID, DEFAULT_PHASE_OFFSET as f64);
                info.unitId = 0;
                info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
                kResultOk
            }
            3 => {
                info.id = PARAM_OUTPUT_GAIN_ID;
                copy_wstring("Output", &mut info.title);
                copy_wstring("Output", &mut info.shortTitle);
                copy_wstring("dB", &mut info.units);
                info.stepCount = 0;
                info.defaultNormalizedValue =
                    to_normalized(PARAM_OUTPUT_GAIN_ID, DEFAULT_OUTPUT_GAIN_DB as f64);
                info.unitId = 0;
                info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
                kResultOk
            }
            4 => {
                info.id = PARAM_SYNC_DIVISION_ID;
                copy_wstring("Division", &mut info.title);
                copy_wstring("Division", &mut info.shortTitle);
                copy_wstring("", &mut info.units);
                info.stepCount = MAX_SYNC_DIVISION as i32;
                info.defaultNormalizedValue =
                    to_normalized(PARAM_SYNC_DIVISION_ID, DEFAULT_SYNC_DIVISION_INDEX as f64);
                info.unitId = 0;
                info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate;
                kResultOk
            }
            _ => kInvalidArgument,
        }
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

        let plain = from_normalized(id, value_normalized);
        let display = match id {
            PARAM_MIX_ID | PARAM_DEPTH_ID | PARAM_PHASE_OFFSET_ID => {
                format!("{:.0}%", plain * 100.0)
            }
            PARAM_OUTPUT_GAIN_ID => format!("{plain:+.1} dB"),
            PARAM_SYNC_DIVISION_ID => sync_division_label(plain as usize).to_string(),
            _ => String::new(),
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

        let value = match id {
            PARAM_SYNC_DIVISION_ID => {
                if string.is_null() {
                    return kInvalidArgument;
                }
                let len = unsafe { tchar_len(string) };
                let utf16 = unsafe { slice::from_raw_parts(string.cast::<u16>(), len) };
                let Some(parsed) = String::from_utf16(utf16).ok() else {
                    return kInvalidArgument;
                };
                let Some(index) = sync_division_index_from_text(parsed.trim()) else {
                    return kInvalidArgument;
                };
                to_normalized(id, index as f64)
            }
            PARAM_MIX_ID | PARAM_DEPTH_ID | PARAM_PHASE_OFFSET_ID => {
                let Some(parsed) = (unsafe { parse_tchar_f64(string) }) else {
                    return kInvalidArgument;
                };
                to_normalized(id, (parsed / 100.0).clamp(0.0, 1.0))
            }
            _ => {
                let Some(parsed) = (unsafe { parse_tchar_f64(string) }) else {
                    return kInvalidArgument;
                };
                to_normalized(id, parsed)
            }
        };

        unsafe { *value_normalized = value };
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
        let Some(view) =
            ComWrapper::new(HostedVst3View::new(adapter, default_width, default_height))
                .to_com_ptr::<IPlugView>()
        else {
            return ptr::null_mut();
        };
        ComPtr::into_raw(view)
    }
}
