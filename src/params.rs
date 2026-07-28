//! Parameter definitions and shared atomic state for Pump.

use std::ffi::CStr;
use std::fmt::Write as _;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::RwLock;

use toybox::clack_extensions::params::{ParamDisplayWriter, ParamInfoFlags, ParamInfoWriter};
use toybox::clack_plugin::prelude::ClapId;
use toybox::clap::params::{ParamBuilder, ParamSpec};
use toybox::dsp::AtomicF32;

use crate::curve::{
    curve_table_to_editable, default_editable_curve, editable_curve_to_table, CurveNode,
    CurveSegment, EditableCurve, CURVE_TABLE_LEN, MAX_EDITABLE_NODES,
};

mod global_curve_slots;
mod host_api;
mod model;
mod preset_store;
mod runtime_impl;
mod state_codec;

pub use model::*;
pub(crate) use model::{
    curve_near_eq, float_near_eq, normalized_preset_name, sanitize_preset_name,
    seeded_quick_shape_slots, STATE_MAGIC, STATE_VERSION,
};

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use global_curve_slots::with_test_curve_slot_path;
#[cfg(test)]
pub(crate) use host_api::param_flags_for_index;
#[cfg(feature = "vst3")]
pub use host_api::{
    apply_normalized_param_value, clap_id_from_vst3_param_id, normalized_from_plain_value,
    plain_from_normalized_value, vst3_param_info_for_index,
};
pub use host_api::{
    apply_param_event, get_param_value, param_count, text_to_value, value_to_text,
    write_param_info, MAX_SYNC_DIVISION,
};
#[cfg(any(feature = "radiant-gui", feature = "vst3", test))]
pub(crate) use host_api::{format_plain_value_text, parse_plain_value_text};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use preset_store::{
    with_test_persistence_failure, with_test_persistence_path, TestPersistenceFailure,
};
pub use state_codec::{decode_state_payload, encode_state_payload};

#[cfg(test)]
mod state_decode_tests;
#[cfg(test)]
mod tests;
