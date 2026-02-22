use super::*;
#[derive(Default)]
struct PumpVst3Factory;

impl Class for PumpVst3Factory {
    type Interfaces = (IPluginFactory,);
}

impl IPluginFactoryTrait for PumpVst3Factory {
    unsafe fn getFactoryInfo(&self, info: *mut PFactoryInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let info = unsafe { &mut *info };
        copy_cstring("PORTALSURFER", &mut info.vendor);
        copy_cstring("https://github.com/uhx/pump", &mut info.url);
        copy_cstring("support@localhost", &mut info.email);
        info.flags = PFactoryInfo_::FactoryFlags_::kUnicode as int32;

        kResultOk
    }

    unsafe fn countClasses(&self) -> i32 {
        2
    }

    unsafe fn getClassInfo(&self, index: i32, info: *mut PClassInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let info = unsafe { &mut *info };
        match index {
            0 => {
                write_class_info_many(
                    info,
                    PROCESSOR_CID,
                    CATEGORY_AUDIO_MODULE_CLASS,
                    PLUGIN_NAME,
                );
                kResultOk
            }
            1 => {
                write_class_info_many(
                    info,
                    CONTROLLER_CID,
                    CATEGORY_COMPONENT_CONTROLLER_CLASS,
                    PLUGIN_NAME,
                );
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn createInstance(
        &self,
        cid: FIDString,
        iid: FIDString,
        obj: *mut *mut c_void,
    ) -> tresult {
        if cid.is_null() || iid.is_null() || obj.is_null() {
            return kInvalidArgument;
        }

        let class_id = unsafe { *(cid as *const TUID) };
        let instance = match class_id {
            PROCESSOR_CID => {
                let shared = acquire_shared_for_role(SharedRole::Processor);
                ComWrapper::new(PumpVst3Processor::new(shared)).to_com_ptr::<FUnknown>()
            }
            CONTROLLER_CID => {
                let shared = acquire_shared_for_role(SharedRole::Controller);
                ComWrapper::new(PumpVst3Controller::new(shared)).to_com_ptr::<FUnknown>()
            }
            _ => None,
        };

        let Some(instance) = instance else {
            return kInvalidArgument;
        };

        let ptr = instance.as_ptr();
        unsafe { ((*(*ptr).vtbl).queryInterface)(ptr, iid as *mut TUID, obj) }
    }
}

toybox::vst3_plugin_entry!(PumpVst3Factory);
