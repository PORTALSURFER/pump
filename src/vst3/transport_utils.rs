use super::*;
pub(super) fn gui_phase_from_transport(
    transport: TransportState,
    settings: DspSettings,
    fallback: f32,
) -> f32 {
    transport
        .song_pos_beats
        .map(|beats| phase_from_beats(beats, settings.beats_per_cycle, settings.phase_offset))
        .unwrap_or_else(|| fallback.rem_euclid(1.0))
}

pub(super) fn host_beat_phase(transport: TransportState) -> Option<f32> {
    transport
        .song_pos_beats
        .map(|beats| beats.rem_euclid(1.0) as f32)
}

pub(super) fn transport_state_from_vst3_process_context(
    process_context: *mut ProcessContext,
) -> TransportState {
    let Some(process_context) = (unsafe { process_context.as_ref() }) else {
        return TransportState::default();
    };

    let state = process_context.state;
    let tempo_valid = (state & ProcessContext_::StatesAndFlags_::kTempoValid as u32) != 0;
    let project_time_music_valid =
        (state & ProcessContext_::StatesAndFlags_::kProjectTimeMusicValid as u32) != 0;
    let is_playing = (state & ProcessContext_::StatesAndFlags_::kPlaying as u32) != 0;
    TransportState {
        tempo_bpm: if tempo_valid {
            process_context.tempo as f32
        } else {
            120.0
        },
        is_playing,
        song_pos_beats: project_time_music_valid.then_some(process_context.projectTimeMusic),
    }
}
