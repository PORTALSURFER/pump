//! Parameter definitions and shared atomic state for Pump.

use std::ffi::CStr;
use std::fmt::Write as _;
use std::io::Cursor;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::RwLock;

use toybox::clack_extensions::params::{ParamDisplayWriter, ParamInfoFlags, ParamInfoWriter};
use toybox::clack_plugin::prelude::ClapId;
use toybox::clap::params::{ParamBuilder, ParamSpec};
use toybox::dsp::AtomicF32;

use crate::curve::{
    curve_table_to_editable, default_editable_curve, editable_curve_to_table, CurveNode,
    CurveSegment, EditableCurve, CURVE_TABLE_LEN, MAX_EDITABLE_NODES,
};

mod host_api;
mod model;
mod preset_store;
mod runtime_impl;
mod state_codec;

pub use model::*;
pub(crate) use model::{
    curve_near_eq, float_near_eq, normalized_preset_name, sanitize_preset_name, STATE_MAGIC,
    STATE_VERSION,
};

#[cfg(feature = "vst3")]
pub use host_api::{
    apply_normalized_param_value, clap_id_from_vst3_param_id, format_plain_value_text,
    normalized_from_plain_value, parse_plain_value_text, plain_from_normalized_value,
    vst3_param_info_for_index,
};
pub use host_api::{
    apply_param_event, get_param_value, param_count, text_to_value, value_to_text,
    write_param_info, MAX_SYNC_DIVISION,
};
pub use state_codec::{decode_state_payload, encode_state_payload};

#[cfg(test)]
mod state_decode_tests;
#[cfg(test)]
mod tests;
