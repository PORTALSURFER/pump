//! Minimal VST3 adapter for Pump.
//!
//! This adapter shares parameter ranges/state with the CLAP implementation and
//! processes the same gain-envelope DSP core.

use std::ffi::{c_void, CStr};
use std::ptr;
use std::slice;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use toybox::clap::automation::AutomationQueue;
use toybox::vst3::prelude::Steinberg::*;
use toybox::vst3::prelude::*;

use crate::dsp::PumpEngine;
use crate::gui::preferred_window_size;
#[cfg(not(target_os = "macos"))]
use crate::gui::PumpGui;
#[cfg(test)]
use crate::params::PARAM_MIX_NUM;
use crate::params::{
    apply_normalized_param_value, clap_id_from_vst3_param_id, decode_state_payload,
    encode_state_payload, format_plain_value_text, get_param_value, normalized_from_plain_value,
    param_count, parse_plain_value_text, plain_from_normalized_value, vst3_param_info_for_index,
    PumpParams,
};
use crate::plugin_metadata::PLUGIN_NAME;
use crate::sample_automation::{
    dsp_settings_from_params, process_stereo_block, ParamEventSchedule,
};
use crate::transport::{gui_phase_from_transport, gui_transport_telemetry};
use crate::GuiStatus;
use toybox::dsp::TransportState;

const PROCESSOR_CID: TUID = uid(0xE5A9A79F, 0xC4A94392, 0x97A8A8AA, 0xA9A90B3C);
const CONTROLLER_CID: TUID = uid(0xB2EE267A, 0xE4314D5D, 0x96085F7A, 0x51681074);

const STATE_MAGIC: u32 = u32::from_le_bytes(*b"PUMP");
const STATE_VERSION: u32 = 1;

mod param_bridge;
mod processor;
mod shared_state;
mod transport_utils;

use param_bridge::{apply_normalized_param, from_normalized, read_plain_param, to_normalized};
use shared_state::{
    acquire_shared_for_role, release_shared_for_role, PumpVst3Runtime, PumpVst3Shared,
    RuntimeHandoff, SharedRole,
};
use transport_utils::transport_state_from_vst3_process_context;

#[cfg(test)]
use shared_state::{shared_registry, SharedRegistryEntry};

mod controller;
mod factory;
mod gui_adapter;

use controller::PumpVst3Controller;
use gui_adapter::PumpVst3GuiAdapter;
use processor::PumpVst3Processor;

#[cfg(test)]
mod tests;
