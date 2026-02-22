use super::*;

/// Extrapolate normalized phase from an anchor phase and elapsed time.
fn extrapolate_phase(
    anchor_phase: f32,
    frequency_hz: f32,
    is_playing: bool,
    last_update_micros: u64,
    now_micros: u64,
) -> f32 {
    let anchor = anchor_phase.rem_euclid(1.0);
    if !is_playing || now_micros <= last_update_micros {
        return anchor;
    }
    let elapsed_seconds = (now_micros - last_update_micros) as f32 / 1_000_000.0;
    (anchor + elapsed_seconds * frequency_hz.max(0.0)).rem_euclid(1.0)
}

/// Transport telemetry payload mirrored from the audio thread to the GUI.
#[derive(Debug, Copy, Clone)]
pub struct GuiTransportTelemetry {
    /// Whether host playback is currently running.
    pub is_playing: bool,
    /// Whether host beat timeline is currently available.
    pub has_host_beats_timeline: bool,
    /// Normalized host beat phase in `[0, 1)`.
    pub beat_phase: f32,
    /// Host tempo in beats per minute.
    pub tempo_bpm: f32,
    /// Pump cycle length in quarter-note beats.
    pub beats_per_cycle: f32,
}

/// Shared GUI telemetry values updated by the audio thread.
#[derive(Default)]
pub struct GuiStatus {
    phase: AtomicF32,
    gain: AtomicF32,
    is_playing: AtomicBool,
    has_host_beats_timeline: AtomicBool,
    beat_phase: AtomicF32,
    tempo_bpm: AtomicF32,
    cycle_hz: AtomicF32,
    last_update_micros: AtomicU64,
}

impl GuiStatus {
    /// Update telemetry from the latest processed frame.
    pub fn update(&self, phase: f32, gain: f32, transport: GuiTransportTelemetry) {
        let safe_tempo = transport.tempo_bpm.clamp(20.0, 320.0);
        let safe_beats_per_cycle = transport.beats_per_cycle.max(1.0e-4);
        self.phase.store(phase, Ordering::Relaxed);
        self.gain.store(gain, Ordering::Relaxed);
        self.is_playing
            .store(transport.is_playing, Ordering::Relaxed);
        self.has_host_beats_timeline
            .store(transport.has_host_beats_timeline, Ordering::Relaxed);
        self.beat_phase
            .store(transport.beat_phase.rem_euclid(1.0), Ordering::Relaxed);
        self.tempo_bpm.store(safe_tempo, Ordering::Relaxed);
        self.cycle_hz.store(
            (safe_tempo / 60.0) / safe_beats_per_cycle,
            Ordering::Relaxed,
        );
        self.last_update_micros
            .store(monotonic_micros(), Ordering::Relaxed);
    }

    /// Read latest phase value.
    pub fn phase(&self) -> f32 {
        extrapolate_phase(
            self.phase.load(Ordering::Relaxed),
            self.cycle_hz.load(Ordering::Relaxed),
            self.is_playing(),
            self.last_update_micros.load(Ordering::Relaxed),
            monotonic_micros(),
        )
    }

    /// Read latest linear gain value.
    pub fn gain(&self) -> f32 {
        self.gain.load(Ordering::Relaxed).max(0.0)
    }

    /// Read whether host transport is currently playing.
    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }

    /// Read whether host beat timeline is currently available.
    pub fn has_host_beats_timeline(&self) -> bool {
        self.has_host_beats_timeline.load(Ordering::Relaxed)
    }

    /// Read normalized host beat phase in `[0, 1)`.
    pub fn beat_phase(&self) -> f32 {
        extrapolate_phase(
            self.beat_phase.load(Ordering::Relaxed),
            self.tempo_bpm.load(Ordering::Relaxed) / 60.0,
            self.is_playing(),
            self.last_update_micros.load(Ordering::Relaxed),
            monotonic_micros(),
        )
    }

    /// Return whether transport beat blink should currently be lit.
    pub fn transport_beat_blink_active(&self) -> bool {
        const BEAT_FLASH_DUTY: f32 = 0.18;
        if !self.is_playing() {
            return false;
        }
        if self.has_host_beats_timeline() {
            return self.beat_phase() < BEAT_FLASH_DUTY;
        }
        // Fallback activity mode: keep the transport indicator lit while
        // playing when the host does not expose a beat timeline.
        true
    }
}

#[allow(clippy::question_mark)]
impl<'a> PluginGuiImpl for PumpMainThread<'a> {
    toybox::patchbay_clap_gui_callbacks!(
        gui = gui,
        preferred_size = crate::gui::preferred_window_size,
        show = |plugin: &mut Self| {
            plugin.gui.open(
                &plugin.shared.params,
                &plugin.shared.status,
                plugin.shared.automation_queue.clone(),
                host_param_requester(plugin.host),
            )
        }
    );
}

#[cfg(test)]
mod tests {
    use crate::dsp::db_to_linear;
    use std::sync::atomic::Ordering;

    use super::{extrapolate_phase, monotonic_micros, GuiStatus, GuiTransportTelemetry};

    #[test]
    fn db_to_linear_matches_unity_at_zero_db() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn transport_beat_blink_requires_playback_and_uses_timeline_when_available() {
        let status = GuiStatus::default();
        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.05,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        assert!(status.transport_beat_blink_active());

        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.5,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        assert!(!status.transport_beat_blink_active());

        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: false,
                has_host_beats_timeline: true,
                beat_phase: 0.05,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        assert!(!status.transport_beat_blink_active());

        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: false,
                beat_phase: 0.05,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        assert!(status.transport_beat_blink_active());
    }

    #[test]
    fn extrapolate_phase_advances_when_playing() {
        let phase = extrapolate_phase(0.25, 2.0, true, 1_000_000, 1_250_000);
        assert!((phase - 0.75).abs() < 1.0e-6);
    }

    #[test]
    fn extrapolate_phase_holds_when_not_playing() {
        let phase = extrapolate_phase(0.25, 2.0, false, 1_000_000, 2_000_000);
        assert!((phase - 0.25).abs() < 1.0e-6);
    }

    #[test]
    fn gui_status_phase_holds_when_last_update_timestamp_is_stale_future_value() {
        let status = GuiStatus::default();
        status.update(
            0.42,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.2,
                tempo_bpm: 120.0,
                beats_per_cycle: 1.0,
            },
        );
        status.last_update_micros.store(
            monotonic_micros().saturating_add(1_000_000),
            Ordering::Relaxed,
        );
        let phase = status.phase();
        assert!(
            (phase - 0.42).abs() <= 1.0e-6,
            "future/stale timestamp should hold anchor phase instead of extrapolating"
        );
    }

    #[test]
    fn gui_status_beat_phase_holds_when_last_update_timestamp_is_stale_future_value() {
        let status = GuiStatus::default();
        status.update(
            0.0,
            1.0,
            GuiTransportTelemetry {
                is_playing: true,
                has_host_beats_timeline: true,
                beat_phase: 0.73,
                tempo_bpm: 123.0,
                beats_per_cycle: 1.0,
            },
        );
        status.last_update_micros.store(
            monotonic_micros().saturating_add(1_000_000),
            Ordering::Relaxed,
        );
        let phase = status.beat_phase();
        assert!(
            (phase - 0.73).abs() <= 1.0e-6,
            "future/stale timestamp should hold anchor beat phase instead of extrapolating"
        );
    }
}
