use super::*;
pub(super) fn to_normalized(param_id: ParamID, plain: f64) -> f64 {
    match param_id {
        PARAM_MIX_ID => ParamRange::new(MIN_MIX as f64, MAX_MIX as f64).plain_to_normalized(plain),
        PARAM_DEPTH_ID => {
            ParamRange::new(MIN_DEPTH as f64, MAX_DEPTH as f64).plain_to_normalized(plain)
        }
        PARAM_PHASE_OFFSET_ID => ParamRange::new(MIN_PHASE_OFFSET as f64, MAX_PHASE_OFFSET as f64)
            .plain_to_normalized(plain),
        PARAM_OUTPUT_GAIN_ID => {
            ParamRange::new(MIN_OUTPUT_GAIN_DB as f64, MAX_OUTPUT_GAIN_DB as f64)
                .plain_to_normalized(plain)
        }
        PARAM_SYNC_DIVISION_ID => {
            ParamRange::new(0.0, MAX_SYNC_DIVISION as f64).plain_to_normalized(plain)
        }
        _ => 0.0,
    }
}

pub(super) fn from_normalized(param_id: ParamID, normalized: f64) -> f64 {
    match param_id {
        PARAM_MIX_ID => {
            ParamRange::new(MIN_MIX as f64, MAX_MIX as f64).normalized_to_plain(normalized)
        }
        PARAM_DEPTH_ID => {
            ParamRange::new(MIN_DEPTH as f64, MAX_DEPTH as f64).normalized_to_plain(normalized)
        }
        PARAM_PHASE_OFFSET_ID => ParamRange::new(MIN_PHASE_OFFSET as f64, MAX_PHASE_OFFSET as f64)
            .normalized_to_plain(normalized),
        PARAM_OUTPUT_GAIN_ID => {
            ParamRange::new(MIN_OUTPUT_GAIN_DB as f64, MAX_OUTPUT_GAIN_DB as f64)
                .normalized_to_plain(normalized)
        }
        PARAM_SYNC_DIVISION_ID => ParamRange::new(0.0, MAX_SYNC_DIVISION as f64)
            .normalized_to_plain(normalized)
            .round(),
        _ => 0.0,
    }
}

pub(super) fn read_plain_param(params: &PumpParams, param_id: ParamID) -> f64 {
    match param_id {
        PARAM_MIX_ID => params.mix() as f64,
        PARAM_DEPTH_ID => params.depth() as f64,
        PARAM_PHASE_OFFSET_ID => params.phase_offset() as f64,
        PARAM_OUTPUT_GAIN_ID => params.output_gain_db() as f64,
        PARAM_SYNC_DIVISION_ID => params.sync_division() as f64,
        _ => 0.0,
    }
}

fn apply_plain_param(params: &PumpParams, param_id: ParamID, plain: f64) {
    match param_id {
        PARAM_MIX_ID => params.set_mix(plain as f32),
        PARAM_DEPTH_ID => params.set_depth(plain as f32),
        PARAM_PHASE_OFFSET_ID => params.set_phase_offset(plain as f32),
        PARAM_OUTPUT_GAIN_ID => params.set_output_gain_db(plain as f32),
        PARAM_SYNC_DIVISION_ID => params.set_sync_division(plain as f32),
        _ => {}
    }
}

pub(super) fn apply_normalized_param(params: &PumpParams, param_id: ParamID, normalized: f64) {
    let plain = from_normalized(param_id, normalized);
    apply_plain_param(params, param_id, plain);
}
