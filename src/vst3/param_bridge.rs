use super::*;

pub(super) fn to_normalized(param_id: ParamID, plain: f64) -> f64 {
    clap_id_from_vst3_param_id(param_id)
        .and_then(|id| normalized_from_plain_value(id, plain))
        .unwrap_or(0.0)
}

pub(super) fn from_normalized(param_id: ParamID, normalized: f64) -> f64 {
    clap_id_from_vst3_param_id(param_id)
        .and_then(|id| plain_from_normalized_value(id, normalized))
        .unwrap_or(0.0)
}

pub(super) fn read_plain_param(params: &PumpParams, param_id: ParamID) -> f64 {
    clap_id_from_vst3_param_id(param_id)
        .and_then(|id| get_param_value(params, id))
        .unwrap_or(0.0)
}

pub(super) fn apply_normalized_param(params: &PumpParams, param_id: ParamID, normalized: f64) {
    let Some(clap_id) = clap_id_from_vst3_param_id(param_id) else {
        return;
    };
    let _applied = apply_normalized_param_value(params, clap_id, normalized);
}
