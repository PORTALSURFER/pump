use super::*;

pub(super) fn transport_state_from_vst3_process_context(
    process_context: *mut ProcessContext,
) -> TransportState {
    let Some(process_context) = (unsafe { process_context.as_ref() }) else {
        return TransportState::default();
    };

    let state = process_context.state;
    let tempo_valid = (state & ProcessContext_::StatesAndFlags_::kTempoValid) != 0;
    let project_time_music_valid =
        (state & ProcessContext_::StatesAndFlags_::kProjectTimeMusicValid) != 0;
    let is_playing = (state & ProcessContext_::StatesAndFlags_::kPlaying) != 0;
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
